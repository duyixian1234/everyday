//! WebDAV device sync module ([D001](../../../docs/adr/D001-webdav-file-sync.md) – [D003](../../../docs/adr/D003-auto-sync-cli-boundary.md)).
//!
//! `everyday sync` bidirectionally syncs the five user data files
//! (`bookmark-<acct>.db` / `note-<acct>.db` / `todo-<acct>.db` / `memory.db` /
//! `config.toml`) against a WebDAV directory (default: Jianguoyun). Derived
//! caches (mail_cache / rss-items / timeline) are never synced.
//!
//! - Engine (change detection / LWW / conflict copies): `engine.rs`
//! - Snapshot & hashing: `snapshot.rs`; state: `state.rs`; client: `client.rs`
//! - Authentication goes through the `auth` module (keyring
//!   `everyday/webdav/<account>`), see [R013](../../../docs/adr/R013-auth-module-consolidation.md) /
//!   [R015](../../../docs/adr/R015-auth-credential-io.md).
//! - Auto-sync (opt-in) pushes changed files after write commands; query
//!   paths never sync ([L005](../../../docs/adr/L005-no-auto-sync.md) / [D003](../../../docs/adr/D003-auto-sync-cli-boundary.md)).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::sync::client::ReqwestClient;
use crate::modules::{Executor, ModuleArgSpec, Output};
use crate::util::args::parse_simple_args;

pub mod client;
pub mod engine;
pub mod snapshot;
pub mod state;

use engine::{SyncAction, SyncFile, SyncOptions};

/// One sync namespace's canonical remote names are derived from the config:
/// every note/todo/bookmark account DB (with its `db_path` override resolved
/// to a local path but the canonical remote name preserved), `memory.db`, and
/// `config.toml`. The manifest is **complete** — files whose local DB has not
/// been created yet are still listed; the engine treats a missing local file
/// as "pull if the remote has it, skip otherwise" (D002: a fresh device must
/// be able to pull every remote file).
pub fn build_file_manifest(config: &Config) -> Result<Vec<SyncFile>> {
    let mut files = Vec::new();

    for module in ["bookmark", "note", "todo"] {
        let accounts: Vec<(String, Option<String>)> = match module {
            "bookmark" => config
                .bookmark
                .accounts
                .iter()
                .map(|a| (a.name.clone(), a.db_path.clone()))
                .collect(),
            "note" => config
                .note
                .accounts
                .iter()
                .map(|a| (a.name.clone(), a.db_path.clone()))
                .collect(),
            _ => config
                .todo
                .accounts
                .iter()
                .map(|a| (a.name.clone(), a.db_path.clone()))
                .collect(),
        };
        for (account, db_path) in accounts {
            let path =
                crate::modules::local::resolve_db_path(module, &account, db_path.as_deref())?;
            files.push(SyncFile {
                local_path: path,
                remote_name: format!("{module}-{account}.db"),
                is_db: true,
            });
        }
    }

    let data_dir = data_dir()?;
    files.push(SyncFile {
        local_path: data_dir.join("memory.db"),
        remote_name: "memory.db".into(),
        is_db: true,
    });

    files.push(SyncFile {
        local_path: Config::config_path()?,
        remote_name: "config.toml".into(),
        is_db: false,
    });

    Ok(files)
}

/// Resolve a canonical remote name to the local file it maps to.
///
/// Used by the fresh-device pull path ([D002](../../../docs/adr/D002-snapshot-hash-state.md)):
/// the local manifest is nearly empty on a new device (no accounts configured,
/// no DBs created), so the pull set is derived from the remote listing instead.
/// `db_path` overrides from the config are honored when the account exists
/// locally; otherwise the default `<config_dir>/everyday/<module>-<account>.db`
/// convention applies (the same default `resolve_db_path` uses).
/// Names outside the sync namespace return `None`.
pub(crate) fn resolve_remote_target(config: &Config, remote_name: &str) -> Option<SyncFile> {
    match remote_name {
        "memory.db" => data_dir().ok().map(|d| SyncFile {
            local_path: d.join("memory.db"),
            remote_name: remote_name.to_string(),
            is_db: true,
        }),
        "config.toml" => Config::config_path().ok().map(|p| SyncFile {
            local_path: p,
            remote_name: remote_name.to_string(),
            is_db: false,
        }),
        _ => {
            for module in ["bookmark", "note", "todo"] {
                let prefix = format!("{module}-");
                if let Some(account) = remote_name
                    .strip_prefix(&prefix)
                    .and_then(|r| r.strip_suffix(".db"))
                {
                    let override_path = account_db_override(config, module, account);
                    let path = crate::modules::local::resolve_db_path(
                        module,
                        account,
                        override_path.as_deref(),
                    )
                    .ok()?;
                    return Some(SyncFile {
                        local_path: path,
                        remote_name: remote_name.to_string(),
                        is_db: true,
                    });
                }
            }
            None
        }
    }
}

/// `db_path` override of the named account in `module`, if the account exists.
fn account_db_override(config: &Config, module: &str, account: &str) -> Option<String> {
    match module {
        "bookmark" => config
            .bookmark
            .accounts
            .iter()
            .find(|a| a.name == account)
            .and_then(|a| a.db_path.clone()),
        "note" => config
            .note
            .accounts
            .iter()
            .find(|a| a.name == account)
            .and_then(|a| a.db_path.clone()),
        _ => config
            .todo
            .accounts
            .iter()
            .find(|a| a.name == account)
            .and_then(|a| a.db_path.clone()),
    }
}

/// `dirs::config_dir()/everyday` — where memory.db and the local DBs live.
fn data_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday"))
}

/// True when the local `config.toml` parses to a shell — no non-webdav
/// accounts at all. First sync then pulls instead of letting LWW clobber the
/// remote config with a fresh empty template (D002).
pub fn is_empty_shell_config() -> bool {
    Config::load_or_default()
        .map(|c| is_shell_config(&c))
        .unwrap_or(false)
}

/// Pure decision: does the config have zero non-webdav accounts/feeds?
pub fn is_shell_config(cfg: &Config) -> bool {
    let account_count = cfg.mail.accounts.len()
        + cfg.calendar.accounts.len()
        + cfg.note.accounts.len()
        + cfg.todo.accounts.len()
        + cfg.bookmark.accounts.len()
        + cfg.rss.feeds.len();
    account_count == 0
}

/// Write actions that touch local data (auto_sync hook target, D003).
pub fn is_write_action(module: &str, action: &str) -> bool {
    matches!(
        (module, action),
        ("bookmark", "add")
            | ("memory", "add" | "delete")
            | ("note", "create" | "append" | "update")
            | ("todo", "add" | "start" | "complete" | "delete")
            | ("cal", "add" | "delete")
            | ("config", "set")
    )
}

/// Best-effort auto push after a successful write command ([D003](../../../docs/adr/D003-auto-sync-cli-boundary.md)).
///
/// Fires for **every** webdav account with `auto_sync = true`. Failures are
/// surfaced as a warning line (text stderr / JSON `_warning`) and never change
/// the command's exit code.
pub async fn auto_sync_after_write(config: Arc<Config>) {
    let accounts: Vec<_> = config
        .webdav
        .accounts
        .iter()
        .filter(|a| a.auto_sync)
        .collect();
    if accounts.is_empty() {
        return;
    }
    for account in accounts {
        let result = run_push_only(&config, account).await;
        let (status, detail) = match result {
            Ok(actions) => {
                let pushed = actions
                    .iter()
                    .filter(|a| matches!(a, SyncAction::Push { .. }))
                    .count();
                if pushed == 0 {
                    continue;
                }
                ("auto_sync_pushed", format!("{pushed} file(s) pushed"))
            }
            Err(e) => ("auto_sync_failed", e.message()),
        };
        if crate::util::json_mode::is_json() {
            let warn = serde_json::json!({ "_warning": status, "message": detail });
            eprintln!("{warn}");
        } else {
            eprintln!("warning: {status}: {detail}");
        }
    }
}

/// Shared bootstrap for a sync run: keyring credential → client, file
/// manifest, and the persisted sync state.
fn prepare_sync(
    config: &Config,
    account: &crate::config::WebdavAccount,
) -> Result<(ReqwestClient, Vec<SyncFile>, state::SyncState)> {
    let secret =
        crate::modules::auth::get_credential_with_user("webdav", &account.name, &account.username)?;
    let client = ReqwestClient::new(&account.username, &secret)?;
    let files = build_file_manifest(config)?;
    let st = state::load(&state::state_path()?)?;
    Ok((client, files, st))
}

/// Push-only execution (auto_sync / `--push-only`).
async fn run_push_only(
    config: &Config,
    account: &crate::config::WebdavAccount,
) -> Result<Vec<SyncAction>> {
    let (client, files, mut st) = prepare_sync(config, account)?;
    let opts = SyncOptions {
        dir_url: account.url.clone(),
        force: false,
        empty_shell_config: false,
        now_utc: chrono::Utc::now(),
    };
    let actions = engine::push_changed(&client, &files, &mut st, &opts).await?;
    state::save(&state::state_path()?, &st)?;
    Ok(actions)
}

/// The `sync` module's executor.
pub struct SyncModule {
    config: Arc<Config>,
}

impl SyncModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for SyncModule {
    fn description(&self) -> &'static str {
        "Cross-device file sync via WebDAV (bookmark/note/todo/memory/config)."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec};
        static ACTIONS: &[ActionArgSpec] = &[crate::modules::ActionArgSpec {
            name: "sync",
            description: "双向同步本机数据文件到 WebDAV（先拉后推）",
            usage: "everyday sync [--push-only|--pull-only|--force] [--account NAME]",
            args: &[
                ArgSpec {
                    name: "push-only",
                    help: "只推（上传本地变更）",
                    kind: ArgKind::Bool,
                },
                ArgSpec {
                    name: "pull-only",
                    help: "只拉（下载远程变更）",
                    kind: ArgKind::Bool,
                },
                ArgSpec {
                    name: "force",
                    help: "忽略 sync-state 全量重传",
                    kind: ArgKind::Bool,
                },
            ],
            positional: crate::modules::Positional::None,
        }];
        ModuleArgSpec {
            name: "sync",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        args: &[String],
        _ctx: &crate::shared::request_context::RequestContext,
    ) -> Result<Output> {
        if action != "sync" {
            return Err(AgentError::UnknownAction(format!("sync {action}")));
        }
        let (flags, _positional) = parse_simple_args(args);
        let push_only = flags.contains_key("push-only");
        let pull_only = flags.contains_key("pull-only");
        let force = flags.contains_key("force");
        if push_only && pull_only {
            return Err(AgentError::InvalidArgument(
                "--push-only and --pull-only are mutually exclusive".into(),
            ));
        }

        let account = self
            .config
            .webdav_account(flags.get("account").map(String::as_str))?;
        let (client, files, mut st) = prepare_sync(&self.config, account)?;
        let opts = SyncOptions {
            dir_url: account.url.clone(),
            force,
            empty_shell_config: is_empty_shell_config(),
            now_utc: chrono::Utc::now(),
        };

        let (actions, first_sync_direction) = if push_only {
            let a = engine::push_changed(&client, &files, &mut st, &opts).await?;
            (a, None)
        } else if pull_only {
            let a = engine::pull_only(&client, &files, &mut st, &opts).await?;
            (a, None)
        } else {
            let resolve_remote = |name: &str| resolve_remote_target(&self.config, name);
            let out = engine::run_sync(&client, &files, &mut st, &opts, &resolve_remote).await?;
            (out.actions, out.first_sync_direction)
        };
        state::save(&state::state_path()?, &st)?;

        render(&actions, account.url.as_str(), first_sync_direction)
    }
}

/// Render sync actions to text lines (with a summary) or a JSON object with
/// per-file `name`/`detail` fields, action counts, and the first-sync
/// direction when the run was an unambiguous first sync.
fn render(actions: &[SyncAction], dir_url: &str, first_sync: Option<&str>) -> Result<Output> {
    let pushed = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::Push { .. }))
        .count();
    let pulled = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::Pull { .. }))
        .count();
    let skipped = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::Skip { .. }))
        .count();
    let conflicts = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::Conflict { .. }))
        .count();

    if crate::util::json_mode::is_json() {
        let arr: Vec<serde_json::Value> = actions
            .iter()
            .map(|a| match a {
                SyncAction::Push { name } => serde_json::json!({
                    "name": name, "action": "push",
                }),
                SyncAction::Pull { name } => serde_json::json!({
                    "name": name, "action": "pull",
                }),
                SyncAction::Skip { name, reason } => serde_json::json!({
                    "name": name, "action": "skip", "detail": reason,
                }),
                SyncAction::Conflict {
                    name,
                    winner,
                    conflict_copy,
                } => serde_json::json!({
                    "name": name, "action": "conflict", "winner": winner,
                    "conflict_copy": conflict_copy,
                }),
            })
            .collect();
        let mut out = serde_json::json!({
            "remote": dir_url,
            "files": arr,
            "pushed": pushed,
            "pulled": pulled,
            "skipped": skipped,
            "conflicts": conflicts,
        });
        if let Some(d) = first_sync {
            out["first_sync"] = serde_json::json!(d);
        }
        Ok(Output::Json(out))
    } else {
        let mut lines = Vec::new();
        for a in actions {
            let line = match a {
                SyncAction::Push { name } => format!("push    {name}"),
                SyncAction::Pull { name } => format!("pull    {name}"),
                SyncAction::Skip { name, reason } => format!("skip    {name}  ({reason})"),
                SyncAction::Conflict {
                    name,
                    winner,
                    conflict_copy,
                } => {
                    format!("conflict {name}  ({winner} won; copy: {conflict_copy})")
                }
            };
            lines.push(line);
        }
        lines.push(format!(
            "summary: {pushed} pushed, {pulled} pulled, {skipped} skipped, {conflicts} conflicts"
        ));
        Ok(Output::text(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_action_whitelist() {
        assert!(is_write_action("bookmark", "add"));
        assert!(is_write_action("memory", "add"));
        assert!(is_write_action("memory", "delete"));
        assert!(is_write_action("note", "create"));
        assert!(is_write_action("note", "append"));
        assert!(is_write_action("note", "update"));
        assert!(is_write_action("todo", "complete"));
        assert!(is_write_action("cal", "add"));
        assert!(is_write_action("config", "set"));
        // Read / query actions never auto-sync (L005).
        assert!(!is_write_action("memory", "get"));
        assert!(!is_write_action("bookmark", "list"));
        assert!(!is_write_action("timeline", "sync"));
        assert!(!is_write_action("mail", "list"));
        assert!(!is_write_action("sync", "sync"));
    }

    #[test]
    fn manifest_includes_existing_db_path_override() {
        // A db_path override pointing at a real file must be included under
        // its canonical remote name — even though the real config dir may or
        // may not hold user data (this test must not depend on the host).
        let tmp = std::env::temp_dir().join(format!(
            "everyday-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let db = tmp.join("notes.db");
        std::fs::write(&db, b"x").unwrap();
        // TOML basic strings treat `\U` as a unicode escape; escape backslashes.
        let db_toml = db.display().to_string().replace('\\', "\\\\");
        let cfg: Config = toml::from_str(&format!(
            r#"
[[todo.accounts]]
name = "personal"
db_path = "{}"
"#,
            db_toml
        ))
        .unwrap();
        let manifest = build_file_manifest(&cfg).unwrap();
        let entry = manifest
            .iter()
            .find(|f| f.remote_name == "todo-personal.db")
            .expect("todo DB with db_path override must be in the manifest");
        assert_eq!(entry.local_path, db);
        assert!(entry.is_db);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn manifest_is_complete_even_without_local_files() {
        // The manifest is complete: accounts whose DBs have not been created
        // locally (e.g. a fresh device) are still listed so the engine can
        // pull them from the remote (D002). Their paths must not exist yet.
        let cfg: Config = toml::from_str(
            r#"
[[bookmark.accounts]]
name = "not-created-yet"

[[note.accounts]]
name = "not-created-yet"

[[todo.accounts]]
name = "not-created-yet"
"#,
        )
        .unwrap();
        let manifest = build_file_manifest(&cfg).unwrap();
        for module in ["bookmark", "note", "todo"] {
            let entry = manifest
                .iter()
                .find(|f| f.remote_name == format!("{module}-not-created-yet.db"))
                .unwrap_or_else(|| panic!("manifest must list {module}-not-created-yet.db"));
            assert!(
                !entry.local_path.exists(),
                "{} must not exist on this host",
                entry.local_path.display()
            );
            assert!(entry.is_db);
        }
        // memory.db + config.toml are always part of the namespace.
        assert!(manifest.iter().any(|f| f.remote_name == "memory.db"));
        assert!(manifest.iter().any(|f| f.remote_name == "config.toml"));
    }

    #[test]
    fn resolve_remote_target_maps_canonical_names() {
        let cfg: Config = toml::from_str(
            r#"
[[todo.accounts]]
name = "personal"
"#,
        )
        .unwrap();

        let mem = resolve_remote_target(&cfg, "memory.db").unwrap();
        assert_eq!(mem.remote_name, "memory.db");
        assert!(mem.is_db);
        assert!(mem.local_path.to_string_lossy().ends_with("memory.db"));

        let cfg_f = resolve_remote_target(&cfg, "config.toml").unwrap();
        assert!(!cfg_f.is_db);
        assert!(cfg_f.local_path.to_string_lossy().ends_with("config.toml"));

        // Account DB resolves to the default config-dir convention.
        let todo = resolve_remote_target(&cfg, "todo-personal.db").unwrap();
        assert_eq!(todo.remote_name, "todo-personal.db");
        assert!(todo.is_db);
        assert!(
            todo.local_path
                .to_string_lossy()
                .ends_with("todo-personal.db")
        );

        // Unknown names are not part of the sync namespace.
        assert!(resolve_remote_target(&cfg, "notes.txt").is_none());
        assert!(resolve_remote_target(&cfg, "timeline.db").is_none());
        assert!(resolve_remote_target(&cfg, "other-personal.db").is_none());
    }

    #[test]
    fn resolve_remote_target_honors_db_path_override() {
        let tmp = std::env::temp_dir().join(format!(
            "everyday-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let custom = tmp.join("custom-note.db");
        let cfg: Config = toml::from_str(&format!(
            r#"
[[note.accounts]]
name = "work"
db_path = "{}"
"#,
            custom.display().to_string().replace('\\', "\\\\")
        ))
        .unwrap();
        let resolved = resolve_remote_target(&cfg, "note-work.db").unwrap();
        assert_eq!(resolved.local_path, custom);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn shell_config_detection() {
        let shell: Config = toml::from_str(
            r#"
[[webdav.accounts]]
name = "personal"
url = "https://dav.example.com/x"
username = "u"
"#,
        )
        .unwrap();
        assert!(is_shell_config(&shell));

        let real: Config = toml::from_str(
            r#"
[[bookmark.accounts]]
name = "personal"

[[rss.feeds]]
name = "hn"
url = "https://hnrss.org/frontpage"
"#,
        )
        .unwrap();
        assert!(!is_shell_config(&real));

        let empty = Config::default();
        assert!(is_shell_config(&empty));
    }

    #[test]
    fn render_json_reports_counts() {
        crate::util::json_mode::set_json_mode(true);
        let actions = vec![
            SyncAction::Push {
                name: "memory.db".into(),
            },
            SyncAction::Pull {
                name: "config.toml".into(),
            },
            SyncAction::Skip {
                name: "a.db".into(),
                reason: "unchanged".into(),
            },
            SyncAction::Conflict {
                name: "b.db".into(),
                winner: "local".into(),
                conflict_copy: "b.conflict-20260809T120000Z.db".into(),
            },
        ];
        let out = render(
            &actions,
            "https://dav.example.com/everyday",
            Some("pull_all"),
        )
        .unwrap();
        crate::util::json_mode::set_json_mode(false);
        match out {
            Output::Json(v) => {
                assert_eq!(v["pushed"], 1);
                assert_eq!(v["pulled"], 1);
                assert_eq!(v["skipped"], 1);
                assert_eq!(v["conflicts"], 1);
                assert_eq!(v["first_sync"], "pull_all");
                assert_eq!(v["files"][0]["name"], "memory.db");
                assert_eq!(v["files"][0]["action"], "push");
                assert_eq!(v["files"][2]["detail"], "unchanged");
                assert_eq!(v["files"][3]["winner"], "local");
            }
            _ => panic!("expected Json output"),
        }
    }

    #[test]
    fn render_text_includes_summary() {
        crate::util::json_mode::set_json_mode(false);
        let actions = vec![
            SyncAction::Push {
                name: "memory.db".into(),
            },
            SyncAction::Pull {
                name: "config.toml".into(),
            },
        ];
        let out = render(&actions, "https://dav.example.com/everyday", None).unwrap();
        match out {
            Output::Text(t) => {
                assert!(t.contains("push    memory.db"));
                assert!(t.contains("pull    config.toml"));
                assert!(t.contains("summary: 1 pushed, 1 pulled, 0 skipped, 0 conflicts"));
            }
            _ => panic!("expected Text output"),
        }
    }
}
