//! Sync engine: per-file change detection, LWW arbitration, conflict copies
//! ([D001](../../../docs/adr/D001-webdav-file-sync.md) /
//! [D002](../../../docs/adr/D002-snapshot-hash-state.md)).
//!
//! Per file the engine compares three facts:
//! - the local content hash (of a `VACUUM INTO` snapshot for DBs),
//! - the previously-recorded state (`sync-state.json`),
//! - the remote ETag / Last-Modified from PROPFIND.
//!
//! Decision matrix (non-first, non-force) — [D002](../../../docs/adr/D002-snapshot-hash-state.md):
//! ```text
//! local_hash == state.local_hash  &&  remote_etag == state.remote_etag  → Skip
//! local_hash == state.local_hash  &&  remote_etag != state.remote_etag  → Pull
//! local_hash != state.local_hash  &&  remote_etag == state.remote_etag  → Push
//! else (both changed, or no state)                                      → LWW via mtimes
//! ```
//! The loser of a conflict is preserved as `<stem>.conflict-<UTCts>.<ext>`
//! both locally and on the remote ([D001](../../../docs/adr/D001-webdav-file-sync.md)) —
//! conflict copies are never lost.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::Result;

use super::client::{RemoteEntry, WebdavClient};
use super::snapshot::{rand_suffix, sha256_file, snapshot_db};
use super::state::SyncState;

/// One file in the sync manifest: local path + canonical remote name.
#[derive(Debug, Clone)]
pub struct SyncFile {
    /// Local filesystem path.
    pub local_path: PathBuf,
    /// Canonical remote name (no path separators; e.g. `memory.db`).
    pub remote_name: String,
    /// SQLite DB (needs a VACUUM INTO snapshot) vs plain text.
    pub is_db: bool,
}

/// What the engine did to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Uploaded the local snapshot.
    Push { name: String },
    /// Downloaded and replaced the local file.
    Pull { name: String },
    /// No work needed.
    Skip { name: String, reason: String },
    /// Both sides changed; the winner replaced the loser, and the loser's
    /// content is preserved in `conflict_copy`.
    Conflict {
        name: String,
        winner: String,
        conflict_copy: String,
    },
}

impl SyncAction {
    #[cfg(test)]
    fn name(&self) -> &str {
        match self {
            SyncAction::Push { name } => name,
            SyncAction::Pull { name } => name,
            SyncAction::Skip { name, .. } => name,
            SyncAction::Conflict { name, .. } => name,
        }
    }
}

/// Engine options.
#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Remote directory URL (base for all remote names).
    pub dir_url: String,
    /// Ignore sync-state and re-upload everything local + pull remote-only
    /// files (rebuild-after-corruption / explicit re-sync).
    pub force: bool,
    /// True when the local `config.toml` parses to a shell (no non-webdav
    /// accounts). First sync then pulls instead of letting LWW clobber the
    /// remote config with a fresh empty template (D002).
    pub empty_shell_config: bool,
    /// Timestamp for conflict-copy names (UTC, injected for tests).
    pub now_utc: chrono::DateTime<chrono::Utc>,
}

/// Run the sync. `state` is mutated in place; the caller persists it.
///
/// `resolve_remote` maps a canonical remote name to the local file it should
/// land in. It is only consulted by the fresh-device pull path (see
/// [`FirstMode::PullAll`]); the per-file path works off `files` directly.
///
/// Returns per-file actions plus the first-sync direction (when the run was a
/// first sync with an unambiguous direction) for display.
pub async fn run_sync(
    client: &dyn WebdavClient,
    files: &[SyncFile],
    state: &mut SyncState,
    opts: &SyncOptions,
    resolve_remote: &(dyn Fn(&str) -> Option<SyncFile> + Sync),
) -> Result<SyncOutcome> {
    client.ensure_dir(&opts.dir_url).await?;
    let remote = client.list(&opts.dir_url).await?;
    let remote_map: HashMap<String, RemoteEntry> =
        remote.into_iter().map(|e| (e.name.clone(), e)).collect();

    let is_first = state.remote_url != opts.dir_url || state.files.is_empty();
    let remote_empty = remote_map.is_empty();

    // First-sync direction detection (D002): empty remote → push everything;
    // remote non-empty + local config is a shell → pull everything (a fresh
    // device must not clobber the remote config with its empty template);
    // otherwise fall through to per-file LWW.
    let first_mode = if !is_first || opts.force {
        FirstMode::PerFile
    } else if remote_empty {
        FirstMode::PushAll
    } else if opts.empty_shell_config {
        FirstMode::PullAll
    } else {
        FirstMode::PerFile
    };

    let first_sync_direction = match first_mode {
        FirstMode::PushAll => Some("push_all"),
        FirstMode::PullAll => Some("pull_all"),
        FirstMode::PerFile => None,
    };

    let tmp_dir = tmp_dir_for(&opts.dir_url);
    std::fs::create_dir_all(&tmp_dir)?;

    let mut actions = Vec::new();
    if matches!(first_mode, FirstMode::PullAll) {
        // Fresh-device initialization: pull **every remote file** that maps to
        // a canonical local path (D002: "本地文件全为默认模板 → 拉远程"). The
        // local manifest is nearly empty on a new device (no accounts, no DBs),
        // so the pull set comes from the remote listing — not from `files`.
        for (name, entry) in &remote_map {
            let Some(file) = resolve_remote(name) else {
                continue; // not part of the sync namespace (foreign file)
            };
            actions.push(pull_one(client, &file, entry, state, opts, &tmp_dir).await?);
        }
    } else {
        for file in files {
            let local_exists = file.local_path.exists();
            let remote_entry = remote_map.get(&file.remote_name);

            let action = match first_mode {
                FirstMode::PushAll => {
                    if local_exists {
                        push_action(client, file, &tmp_dir, state, opts).await?
                    } else {
                        SyncAction::Skip {
                            name: file.remote_name.clone(),
                            reason: "local missing".into(),
                        }
                    }
                }
                FirstMode::PerFile => {
                    per_file(client, file, remote_entry, state, &tmp_dir, opts).await?
                }
                FirstMode::PullAll => unreachable!("handled above"),
            };
            actions.push(action);
        }
    }

    // Best-effort cleanup of snapshot temp files.
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Some servers (e.g. Jianguoyun) do not return an ETag header on PUT, so
    // the state recorded from the PUT response has `remote_etag = None`. A
    // later PROPFIND does return one, which would make the next run see
    // "remote changed" and pull back what we just pushed. Re-list the
    // directory once and correct the pushed files' metadata from PROPFIND.
    let pushed: Vec<String> = actions
        .iter()
        .filter(|a| {
            matches!(a, SyncAction::Push { .. })
                || matches!(a, SyncAction::Conflict { winner, .. } if winner == "local")
        })
        .map(|a| match a {
            SyncAction::Push { name } => name.clone(),
            SyncAction::Conflict { name, .. } => name.clone(),
            _ => unreachable!(),
        })
        .collect();
    refresh_remote_metadata(client, &opts.dir_url, state, &pushed).await?;

    // Bind the state to this remote URL so the next run is not "first" again.
    state.remote_url = opts.dir_url.clone();
    Ok(SyncOutcome {
        actions,
        first_sync_direction,
    })
}

/// Re-read the remote directory and overwrite the pushed files' recorded
/// ETag / Last-Modified with the PROPFIND values (PUT responses often lack an
/// ETag header; PROPFIND is authoritative for change detection).
async fn refresh_remote_metadata(
    client: &dyn WebdavClient,
    dir_url: &str,
    state: &mut SyncState,
    names: &[String],
) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let remote = client.list(dir_url).await?;
    let map: HashMap<String, RemoteEntry> =
        remote.into_iter().map(|e| (e.name.clone(), e)).collect();
    for name in names {
        if let Some(entry) = map.get(name)
            && let Some(fs) = state.files.get_mut(name)
        {
            fs.remote_etag = entry.etag.clone();
            fs.remote_mtime = entry.last_modified.clone();
        }
    }
    Ok(())
}

/// Result of a bidirectional sync: the per-file actions plus (for display)
/// which first-sync direction was taken, if any.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub actions: Vec<SyncAction>,
    /// `Some("push_all")` / `Some("pull_all")` when the run was a first sync
    /// with an unambiguous direction; `None` otherwise (per-file / force).
    pub first_sync_direction: Option<&'static str>,
}

enum FirstMode {
    PushAll,
    PullAll,
    PerFile,
}

/// Per-file decision (non-first / force path).
async fn per_file(
    client: &dyn WebdavClient,
    file: &SyncFile,
    remote: Option<&RemoteEntry>,
    state: &mut SyncState,
    tmp_dir: &Path,
    opts: &SyncOptions,
) -> Result<SyncAction> {
    let local_exists = file.local_path.exists();

    // `--force`: re-upload everything local, pull remote-only files (explicit
    // overwrite semantics — no conflict copies, the user asked for it).
    if opts.force {
        return if local_exists {
            push_action(client, file, tmp_dir, state, opts).await
        } else if let Some(r) = remote {
            pull_one(client, file, r, state, opts, tmp_dir).await
        } else {
            Ok(SyncAction::Skip {
                name: file.remote_name.clone(),
                reason: "missing on both sides".into(),
            })
        };
    }

    match (local_exists, remote) {
        (false, None) => Ok(SyncAction::Skip {
            name: file.remote_name.clone(),
            reason: "missing on both sides".into(),
        }),
        // Remote-only → pull regardless of state.
        (false, Some(r)) => pull_one(client, file, r, state, opts, tmp_dir).await,
        // Local-only → push regardless of state.
        (true, None) => push_action(client, file, tmp_dir, state, opts).await,
        (true, Some(r)) => {
            let local_hash = local_hash_of(file, tmp_dir).await?;
            let prev = state.files.get(&file.remote_name);

            let same_local = prev
                .map(|p| p.local_hash.as_deref() == Some(local_hash.as_str()))
                .unwrap_or(false);
            let same_remote = prev
                .map(|p| p.remote_etag.is_some() && p.remote_etag == r.etag)
                .unwrap_or(false);

            if same_local && same_remote {
                Ok(SyncAction::Skip {
                    name: file.remote_name.clone(),
                    reason: "unchanged".into(),
                })
            } else if same_local && !same_remote {
                pull_one(client, file, r, state, opts, tmp_dir).await
            } else if !same_local && same_remote {
                push_action(client, file, tmp_dir, state, opts).await
            } else {
                // Both changed (or no prior state) → LWW via mtimes.
                lww_resolve(client, file, r, &local_hash, tmp_dir, state, opts).await
            }
        }
    }
}

/// Last-Write-Wins arbitration. The loser is preserved as a conflict copy,
/// kept locally **and** uploaded to the remote ([D001](../../../docs/adr/D001-webdav-file-sync.md)).
async fn lww_resolve(
    client: &dyn WebdavClient,
    file: &SyncFile,
    remote: &RemoteEntry,
    local_hash: &str,
    tmp_dir: &Path,
    state: &mut SyncState,
    opts: &SyncOptions,
) -> Result<SyncAction> {
    // Config shell-guard: when the local config.toml is a shell (no non-webdav
    // accounts) the remote config always wins — a fresh device's empty template
    // must never clobber the remote config via mtime LWW
    // ([D002](../../../docs/adr/D002-snapshot-hash-state.md)). No conflict copy:
    // the local shell is a template, not user data.
    if opts.empty_shell_config && file.remote_name == "config.toml" {
        let new_remote = client.get(&opts.dir_url, &file.remote_name).await?;
        replace_local(&file.local_path, &new_remote)?;
        let local_hash = local_hash_of(file, tmp_dir).await?;
        record_after_pull(state, file, &local_hash, remote);
        return Ok(SyncAction::Pull {
            name: file.remote_name.clone(),
        });
    }

    let local_mtime = std::fs::metadata(&file.local_path)
        .and_then(|m| m.modified())
        .ok();
    let remote_mtime = remote
        .last_modified
        .as_deref()
        .and_then(|s| httpdate::parse_http_date(s).ok());

    let local_wins = match (local_mtime, remote_mtime) {
        (Some(l), Some(r)) => l >= r,
        (Some(_), None) => true, // no remote timestamp → prefer the active side
        (None, _) => false,
    };

    let ts = opts.now_utc.format("%Y%m%dT%H%M%SZ").to_string();
    let conflict_name = conflict_name_for(&file.remote_name, &ts);

    if local_wins {
        // Local is newer: push the local snapshot; the remote version is the
        // loser → keep it locally and upload it so the other device sees it.
        let old_remote = client.get(&opts.dir_url, &file.remote_name).await?;
        let entry = push_one(client, file, tmp_dir, state, opts).await?;
        write_conflict_local(file, &old_remote, &ts)?;
        client
            .put(&opts.dir_url, &conflict_name, &old_remote)
            .await?;
        // State now reflects the pushed local content.
        record_after_push(state, file, local_hash, &entry);
        Ok(SyncAction::Conflict {
            name: file.remote_name.clone(),
            winner: "local".into(),
            conflict_copy: conflict_name,
        })
    } else {
        // Remote is newer: pull the remote version; the local side is the
        // loser → snapshot it, keep the copy locally and upload it.
        let new_remote = client.get(&opts.dir_url, &file.remote_name).await?;
        let local_snapshot = local_bytes(file, tmp_dir).await?;
        replace_local(&file.local_path, &new_remote)?;
        write_conflict_local(file, &local_snapshot, &ts)?;
        client
            .put(&opts.dir_url, &conflict_name, &local_snapshot)
            .await?;
        let local_hash = local_hash_of(file, tmp_dir).await?;
        record_after_pull(state, file, &local_hash, remote);
        Ok(SyncAction::Conflict {
            name: file.remote_name.clone(),
            winner: "remote".into(),
            conflict_copy: conflict_name,
        })
    }
}

/// Push the local file (snapshot for DBs); returns the server metadata.
async fn push_one(
    client: &dyn WebdavClient,
    file: &SyncFile,
    tmp_dir: &Path,
    state: &mut SyncState,
    opts: &SyncOptions,
) -> Result<RemoteEntry> {
    let body = local_bytes(file, tmp_dir).await?;
    let entry = client.put(&opts.dir_url, &file.remote_name, &body).await?;
    record_after_push(state, file, &sha256_bytes(&body), &entry);
    Ok(entry)
}

/// Push the local file, discard the server metadata, and report `Push`.
async fn push_action(
    client: &dyn WebdavClient,
    file: &SyncFile,
    tmp_dir: &Path,
    state: &mut SyncState,
    opts: &SyncOptions,
) -> Result<SyncAction> {
    let _ = push_one(client, file, tmp_dir, state, opts).await?;
    Ok(SyncAction::Push {
        name: file.remote_name.clone(),
    })
}

/// Pull the remote file, atomically replace the local path, and record state.
///
/// The recorded local hash uses the same snapshot convention as change
/// detection (`local_hash_of`): for DBs that means VACUUM INTO snapshotting
/// the pulled file. Recording the raw downloaded bytes instead would mismatch
/// the detection hash and make every pulled DB look "locally changed" on the
/// next run.
async fn pull_one(
    client: &dyn WebdavClient,
    file: &SyncFile,
    remote: &RemoteEntry,
    state: &mut SyncState,
    opts: &SyncOptions,
    tmp_dir: &Path,
) -> Result<SyncAction> {
    let bytes = client.get(&opts.dir_url, &file.remote_name).await?;
    replace_local(&file.local_path, &bytes)?;
    let local_hash = local_hash_of(file, tmp_dir).await?;
    record_after_pull(state, file, &local_hash, remote);
    Ok(SyncAction::Pull {
        name: file.remote_name.clone(),
    })
}

/// Content hash of the local file, snapshotting DBs first (D002).
async fn local_hash_of(file: &SyncFile, tmp_dir: &Path) -> Result<String> {
    if file.is_db {
        let snap = snapshot_db(&file.local_path, tmp_dir).await?;
        let h = sha256_file(&snap)?;
        let _ = std::fs::remove_file(&snap);
        Ok(h)
    } else {
        sha256_file(&file.local_path)
    }
}

/// The bytes to upload for a local file (snapshot for DBs).
async fn local_bytes(file: &SyncFile, tmp_dir: &Path) -> Result<Vec<u8>> {
    if file.is_db {
        let snap = snapshot_db(&file.local_path, tmp_dir).await?;
        let bytes = std::fs::read(&snap)?;
        let _ = std::fs::remove_file(&snap);
        Ok(bytes)
    } else {
        Ok(std::fs::read(&file.local_path)?)
    }
}

/// Atomically replace the local file with `bytes` (temp + rename, D002).
fn replace_local(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Conflict-copy name: `<stem>.conflict-<UTCts>.<ext>` — the original
/// extension stays at the end so the copy is still recognizable as a DB or
/// TOML file (`memory.db` → `memory.conflict-20260809T120000Z.db`,
/// `config.toml` → `config.conflict-20260809T120000Z.toml`).
fn conflict_name_for(remote_name: &str, ts: &str) -> String {
    match remote_name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.conflict-{ts}.{ext}"),
        None => format!("{remote_name}.conflict-{ts}"),
    }
}

/// Write a conflict copy next to the local file (`<stem>.conflict-<ts>.<ext>`).
fn write_conflict_local(file: &SyncFile, bytes: &[u8], ts: &str) -> Result<()> {
    let dir = file.local_path.parent().unwrap_or(Path::new("."));
    let conflict_path = dir.join(conflict_name_for(&file.remote_name, ts));
    std::fs::write(conflict_path, bytes)?;
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Temporary directory for snapshots, keyed by remote URL basename.
fn tmp_dir_for(dir_url: &str) -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    let key = dir_url
        .split('/')
        .rfind(|s| !s.is_empty())
        .unwrap_or("sync");
    // Unique per-invocation subdirectory: run_sync removes its own tmp dir on
    // exit, so concurrent runs (parallel tests, multi-command scripts) must not
    // share one directory — a shared dir gets deleted out from under peers
    // (VACUUM INTO CANTOPEN/IOERR on Unix; masked by file locks on Windows).
    dir.join("everyday")
        .join(".sync-tmp")
        .join(key)
        .join(rand_suffix())
}

// ============ state bookkeeping ============

/// Record state after a push: the local content is authoritative remotely.
fn record_after_push(
    state: &mut SyncState,
    file: &SyncFile,
    local_hash: &str,
    entry: &RemoteEntry,
) {
    let fs = state.files.entry(file.remote_name.clone()).or_default();
    fs.local_hash = Some(local_hash.to_string());
    fs.remote_etag = entry.etag.clone();
    fs.remote_mtime = entry.last_modified.clone();
}

/// Record state after a pull: the local content is now the remote content.
/// `local_hash` is the snapshot-convention hash of the pulled file (caller
/// computes it via `local_hash_of` so it matches change detection).
fn record_after_pull(
    state: &mut SyncState,
    file: &SyncFile,
    local_hash: &str,
    remote: &RemoteEntry,
) {
    let fs = state.files.entry(file.remote_name.clone()).or_default();
    fs.local_hash = Some(local_hash.to_string());
    fs.remote_etag = remote.etag.clone();
    fs.remote_mtime = remote.last_modified.clone();
}

/// Push-only: upload every local file whose content hash differs from the
/// recorded state (used by `--push-only` and auto_sync). Never pulls — remote
/// conflicts are left for the next explicit bidirectional sync.
pub async fn push_changed(
    client: &dyn WebdavClient,
    files: &[SyncFile],
    state: &mut SyncState,
    opts: &SyncOptions,
) -> Result<Vec<SyncAction>> {
    client.ensure_dir(&opts.dir_url).await?;
    let tmp_dir = tmp_dir_for(&opts.dir_url);
    std::fs::create_dir_all(&tmp_dir)?;
    let mut actions = Vec::new();
    for file in files {
        if !file.local_path.exists() {
            actions.push(SyncAction::Skip {
                name: file.remote_name.clone(),
                reason: "local missing".into(),
            });
            continue;
        }
        let hash = local_hash_of(file, &tmp_dir).await?;
        let unchanged = state
            .files
            .get(&file.remote_name)
            .map(|p| p.local_hash.as_deref() == Some(hash.as_str()))
            .unwrap_or(false);
        if unchanged {
            actions.push(SyncAction::Skip {
                name: file.remote_name.clone(),
                reason: "unchanged".into(),
            });
            continue;
        }
        actions.push(push_action(client, file, &tmp_dir, state, opts).await?);
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
    // Correct pushed files' ETags from PROPFIND (PUT responses often carry no
    // ETag header; see `refresh_remote_metadata`).
    let pushed: Vec<String> = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::Push { .. }))
        .map(|a| match a {
            SyncAction::Push { name } => name.clone(),
            _ => unreachable!(),
        })
        .collect();
    refresh_remote_metadata(client, &opts.dir_url, state, &pushed).await?;
    state.remote_url = opts.dir_url.clone();
    Ok(actions)
}

/// Pull-only: download every remote file whose ETag differs from the recorded
/// state, or that is missing locally (used by `--pull-only`). Never pushes.
pub async fn pull_only(
    client: &dyn WebdavClient,
    files: &[SyncFile],
    state: &mut SyncState,
    opts: &SyncOptions,
) -> Result<Vec<SyncAction>> {
    client.ensure_dir(&opts.dir_url).await?;
    let remote = client.list(&opts.dir_url).await?;
    let remote_map: HashMap<String, RemoteEntry> =
        remote.into_iter().map(|e| (e.name.clone(), e)).collect();
    let tmp_dir = tmp_dir_for(&opts.dir_url);
    std::fs::create_dir_all(&tmp_dir)?;
    let mut actions = Vec::new();
    for file in files {
        let Some(entry) = remote_map.get(&file.remote_name) else {
            actions.push(SyncAction::Skip {
                name: file.remote_name.clone(),
                reason: "remote missing".into(),
            });
            continue;
        };
        let unchanged = file.local_path.exists()
            && state
                .files
                .get(&file.remote_name)
                .map(|p| p.remote_etag.is_some() && p.remote_etag == entry.etag)
                .unwrap_or(false);
        if unchanged {
            actions.push(SyncAction::Skip {
                name: file.remote_name.clone(),
                reason: "unchanged".into(),
            });
            continue;
        }
        pull_one(client, file, entry, state, opts, &tmp_dir).await?;
        actions.push(SyncAction::Pull {
            name: file.remote_name.clone(),
        });
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
    state.remote_url = opts.dir_url.clone();
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::client::test_support::MockWebdavClient;
    use crate::modules::sync::state::SyncState;

    const URL: &str = "https://dav.example.com/everyday";
    // Conflict-copy timestamps use the compact UTC form the engine emits
    // (%Y%m%dT%H%M%SZ); the RFC3339 form contains ':' which is illegal in
    // Windows filenames.
    const NOW_COMPACT: &str = "20260809T120000Z";

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-08-09T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn opts(force: bool, empty_shell: bool) -> SyncOptions {
        SyncOptions {
            dir_url: URL.to_string(),
            force,
            empty_shell_config: empty_shell,
            now_utc: now(),
        }
    }

    /// A `resolve_remote` stub that maps every canonical name to `dir/name`
    /// (used by the fresh-device pull tests). `is_db: false` keeps the seeds
    /// plain text — this stub only exercises the pull-set logic; the DB
    /// snapshot path has its own dedicated test.
    fn resolve_in(dir: &Path) -> impl Fn(&str) -> Option<SyncFile> + '_ {
        move |name: &str| {
            Some(SyncFile {
                local_path: dir.join(name),
                remote_name: name.to_string(),
                is_db: false,
            })
        }
    }

    /// A `resolve_remote` stub that resolves nothing (per-file tests never
    /// touch the fresh-device pull path).
    fn resolve_none(_: &str) -> Option<SyncFile> {
        None
    }

    fn tmp_workdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "everyday-engine-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn text_file(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
    }

    fn action_of<'a>(actions: &'a [SyncAction], name: &str) -> &'a SyncAction {
        actions
            .iter()
            .find(|a| a.name() == name)
            .unwrap_or_else(|| panic!("no action for {name}: {actions:?}"))
    }

    fn assert_kind(actions: &[SyncAction], name: &str, kind: &str) {
        let a = action_of(actions, name);
        let actual = match a {
            SyncAction::Push { .. } => "push",
            SyncAction::Pull { .. } => "pull",
            SyncAction::Skip { .. } => "skip",
            SyncAction::Conflict { .. } => "conflict",
        };
        assert_eq!(actual, kind, "file {name}: {actions:?}");
    }

    // ---- first sync ----

    #[tokio::test]
    async fn first_sync_empty_remote_pushes_all() {
        let d = tmp_workdir("first-push");
        let c = MockWebdavClient::default();
        let mut state = SyncState::default();
        let files = vec![
            SyncFile {
                local_path: d.join("memory.db"),
                remote_name: "memory.db".into(),
                is_db: false,
            },
            SyncFile {
                local_path: d.join("config.toml"),
                remote_name: "config.toml".into(),
                is_db: false,
            },
        ];
        text_file(&files[0].local_path, "mem-data");
        text_file(&files[1].local_path, "cfg-data");

        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_eq!(actions.len(), 2);
        assert_kind(&actions, "memory.db", "push");
        assert_kind(&actions, "config.toml", "push");
        assert_eq!(
            c.files.lock().unwrap().get("memory.db").unwrap(),
            b"mem-data"
        );
        assert_eq!(state.remote_url, URL);
        assert!(state.files["memory.db"].local_hash.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn first_sync_remote_full_shell_config_pulls_all() {
        let d = tmp_workdir("first-pull");
        let c = MockWebdavClient::default();
        c.seed("memory.db", b"remote-mem");
        c.seed("config.toml", b"remote-cfg");
        // A remote DB that has no local counterpart at all — the fresh-device
        // pull set comes from the remote listing, not from the local manifest
        // (D002: "本地文件全为默认模板 → 拉远程").
        c.seed("note-personal.db", b"remote-note-db");
        let mut state = SyncState::default();
        let files = vec![
            SyncFile {
                local_path: d.join("memory.db"),
                remote_name: "memory.db".into(),
                is_db: false,
            },
            SyncFile {
                local_path: d.join("config.toml"),
                remote_name: "config.toml".into(),
                is_db: false,
            },
        ];
        // Local config.toml exists but is a shell (empty-slot flag from caller).
        text_file(&files[1].local_path, "[webdav]\n"); // shell template

        let resolve_remote = resolve_in(&d);
        let actions = run_sync(&c, &files, &mut state, &opts(false, true), &resolve_remote)
            .await
            .unwrap()
            .actions;
        assert_eq!(actions.len(), 3);
        assert_kind(&actions, "memory.db", "pull");
        assert_kind(&actions, "config.toml", "pull");
        assert_kind(&actions, "note-personal.db", "pull");
        assert_eq!(std::fs::read(&files[0].local_path).unwrap(), b"remote-mem");
        assert_eq!(std::fs::read(&files[1].local_path).unwrap(), b"remote-cfg");
        // The remote-only DB landed at its canonical default path.
        assert_eq!(
            std::fs::read(d.join("note-personal.db")).unwrap(),
            b"remote-note-db"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn first_sync_both_sides_real_data_uses_lww() {
        let d = tmp_workdir("first-lww");
        let c = MockWebdavClient::with_last_modified("Sun, 09 Aug 2026 12:00:00 GMT");
        c.seed("memory.db", b"remote-mem");
        let mut state = SyncState::default();
        let files = vec![SyncFile {
            local_path: d.join("memory.db"),
            remote_name: "memory.db".into(),
            is_db: false,
        }];
        text_file(&files[0].local_path, "local-mem");

        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "memory.db", "conflict");
        // Conflict copy exists locally and remotely.
        let conflict = format!("memory.conflict-{NOW_COMPACT}.db");
        assert!(d.join(&conflict).exists(), "local conflict copy missing");
        assert!(
            c.files.lock().unwrap().contains_key(&conflict),
            "remote conflict copy missing"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- steady state matrix ----

    #[tokio::test]
    async fn steady_unchanged_skips() {
        let d = tmp_workdir("steady-skip");
        let c = MockWebdavClient::default();
        text_file(&d.join("config.toml"), "cfg-v1");
        c.seed("config.toml", b"cfg-v1");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "config.toml".to_string(),
                super::super::state::FileState {
                    local_hash: Some(sha256_bytes(b"cfg-v1")),
                    remote_etag: c.list(URL).await.unwrap()[0].etag.clone(),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![SyncFile {
            local_path: d.join("config.toml"),
            remote_name: "config.toml".into(),
            is_db: false,
        }];
        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "config.toml", "skip");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn steady_local_changed_pushes() {
        let d = tmp_workdir("steady-push");
        let c = MockWebdavClient::default();
        text_file(&d.join("config.toml"), "cfg-v2");
        c.seed("config.toml", b"cfg-v1");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "config.toml".to_string(),
                super::super::state::FileState {
                    local_hash: Some(sha256_bytes(b"cfg-v1")),
                    remote_etag: c.list(URL).await.unwrap()[0].etag.clone(),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![SyncFile {
            local_path: d.join("config.toml"),
            remote_name: "config.toml".into(),
            is_db: false,
        }];
        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "config.toml", "push");
        assert_eq!(
            c.files.lock().unwrap().get("config.toml").unwrap(),
            b"cfg-v2"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn steady_remote_changed_pulls() {
        let d = tmp_workdir("steady-pull");
        let c = MockWebdavClient::default();
        text_file(&d.join("config.toml"), "cfg-v1");
        c.seed("config.toml", b"cfg-v2");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "config.toml".to_string(),
                super::super::state::FileState {
                    local_hash: Some(sha256_bytes(b"cfg-v1")),
                    remote_etag: Some("\"stale-etag\"".into()),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![SyncFile {
            local_path: d.join("config.toml"),
            remote_name: "config.toml".into(),
            is_db: false,
        }];
        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "config.toml", "pull");
        assert_eq!(std::fs::read(&files[0].local_path).unwrap(), b"cfg-v2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn local_only_pushes_remote_only_pulls() {
        let d = tmp_workdir("one-sided");
        let c = MockWebdavClient::default();
        text_file(&d.join("local-only.txt"), "L");
        c.seed("remote-only.txt", b"R");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: Default::default(),
        };
        let files = vec![
            SyncFile {
                local_path: d.join("local-only.txt"),
                remote_name: "local-only.txt".into(),
                is_db: false,
            },
            SyncFile {
                local_path: d.join("remote-only.txt"),
                remote_name: "remote-only.txt".into(),
                is_db: false,
            },
        ];
        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "local-only.txt", "push");
        assert_kind(&actions, "remote-only.txt", "pull");
        assert_eq!(c.files.lock().unwrap().get("local-only.txt").unwrap(), b"L");
        assert_eq!(std::fs::read(&files[1].local_path).unwrap(), b"R");
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- conflicts ----

    #[tokio::test]
    async fn conflict_local_wins_preserves_remote_copy() {
        let d = tmp_workdir("conflict-local");
        let c = MockWebdavClient::with_last_modified("Sun, 09 Aug 2026 12:00:00 GMT");
        text_file(&d.join("memory.db"), "local-newer");
        c.seed("memory.db", b"remote-older");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "memory.db".to_string(),
                super::super::state::FileState {
                    local_hash: Some("stale".into()),
                    remote_etag: Some("stale".into()),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![SyncFile {
            local_path: d.join("memory.db"),
            remote_name: "memory.db".into(),
            is_db: false,
        }];
        // Make local mtime newer than the remote Last-Modified.
        let remote_ts = httpdate::parse_http_date("Sun, 09 Aug 2026 12:00:00 GMT").unwrap();
        let newer = remote_ts + std::time::Duration::from_secs(3600);
        // filetime crate not available; use std::fs::FileTimes (Rust 1.75+).
        let ft = std::fs::FileTimes::new().set_modified(newer);
        std::fs::File::options()
            .write(true)
            .open(&files[0].local_path)
            .unwrap()
            .set_times(ft)
            .unwrap();

        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "memory.db", "conflict");
        let conflict = format!("memory.conflict-{NOW_COMPACT}.db");
        assert_eq!(std::fs::read(d.join(&conflict)).unwrap(), b"remote-older");
        assert_eq!(
            c.files.lock().unwrap().get(&conflict).unwrap(),
            b"remote-older"
        );
        // Local file is now the pushed local version (unchanged).
        assert_eq!(std::fs::read(&files[0].local_path).unwrap(), b"local-newer");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn conflict_remote_wins_preserves_local_copy() {
        let d = tmp_workdir("conflict-remote");
        let c = MockWebdavClient::with_last_modified("Sun, 09 Aug 2026 12:00:00 GMT");
        text_file(&d.join("memory.db"), "local-older");
        c.seed("memory.db", b"remote-newer");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "memory.db".to_string(),
                super::super::state::FileState {
                    local_hash: Some("stale".into()),
                    remote_etag: Some("stale".into()),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![SyncFile {
            local_path: d.join("memory.db"),
            remote_name: "memory.db".into(),
            is_db: false,
        }];
        // Keep local mtime older than the remote Last-Modified (default file
        // mtime is "now", which is later in real time than 2026-08-09? No —
        // today IS 2026-08-09; force it older explicitly).
        let older = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let ft = std::fs::FileTimes::new().set_modified(older);
        std::fs::File::options()
            .write(true)
            .open(&files[0].local_path)
            .unwrap()
            .set_times(ft)
            .unwrap();

        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "memory.db", "conflict");
        let conflict = format!("memory.conflict-{NOW_COMPACT}.db");
        assert_eq!(std::fs::read(d.join(&conflict)).unwrap(), b"local-older");
        assert_eq!(
            c.files.lock().unwrap().get(&conflict).unwrap(),
            b"local-older"
        );
        // Local file replaced by the remote winner.
        assert_eq!(
            std::fs::read(&files[0].local_path).unwrap(),
            b"remote-newer"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- config shell-guard (LWW) ----

    #[tokio::test]
    async fn shell_config_lww_remote_wins_no_conflict_copy() {
        let d = tmp_workdir("shell-lww");
        let c = MockWebdavClient::with_last_modified("Sun, 09 Aug 2026 12:00:00 GMT");
        // Local config is a shell template; remote config is real data. Both
        // sides changed since state, so this goes to LWW — and the shell guard
        // must make the remote win (pull), with no conflict copy (the local
        // shell is a template, not user data).
        text_file(&d.join("config.toml"), "[webdav]\n");
        c.seed("config.toml", b"real remote config");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "config.toml".to_string(),
                super::super::state::FileState {
                    local_hash: Some("stale".into()),
                    remote_etag: Some("stale".into()),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        // Local mtime must be NEWER than the remote Last-Modified — LWW alone
        // would pick local; the shell guard must override that.
        let remote_ts = httpdate::parse_http_date("Sun, 09 Aug 2026 12:00:00 GMT").unwrap();
        let newer = remote_ts + std::time::Duration::from_secs(3600);
        let ft = std::fs::FileTimes::new().set_modified(newer);
        std::fs::File::options()
            .write(true)
            .open(d.join("config.toml"))
            .unwrap()
            .set_times(ft)
            .unwrap();

        let files = vec![SyncFile {
            local_path: d.join("config.toml"),
            remote_name: "config.toml".into(),
            is_db: false,
        }];
        let actions = run_sync(&c, &files, &mut state, &opts(false, true), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "config.toml", "pull");
        assert_eq!(
            std::fs::read(&files[0].local_path).unwrap(),
            b"real remote config"
        );
        // No conflict copy for the shell template.
        let conflict = format!("config.conflict-{NOW_COMPACT}.toml");
        assert!(
            !d.join(&conflict).exists(),
            "shell guard must not keep a copy"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- force ----

    #[tokio::test]
    async fn force_uploads_local_and_pulls_remote_only() {
        let d = tmp_workdir("force");
        let c = MockWebdavClient::default();
        text_file(&d.join("memory.db"), "local-data");
        c.seed("remote-only.db", b"remote-data");
        let mut state = SyncState {
            remote_url: URL.into(),
            files: [(
                "memory.db".to_string(),
                super::super::state::FileState {
                    local_hash: Some("stale".into()),
                    remote_etag: Some("stale".into()),
                    remote_mtime: None,
                },
            )]
            .into_iter()
            .collect(),
        };
        let files = vec![
            SyncFile {
                local_path: d.join("memory.db"),
                remote_name: "memory.db".into(),
                is_db: false,
            },
            SyncFile {
                local_path: d.join("remote-only.db"),
                remote_name: "remote-only.db".into(),
                is_db: false,
            },
        ];
        let actions = run_sync(&c, &files, &mut state, &opts(true, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "memory.db", "push");
        assert_kind(&actions, "remote-only.db", "pull");
        assert_eq!(
            c.files.lock().unwrap().get("memory.db").unwrap(),
            b"local-data"
        );
        assert_eq!(std::fs::read(&files[1].local_path).unwrap(), b"remote-data");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn push_then_second_sync_skips_when_put_has_no_etag() {
        // Some servers (Jianguoyun) omit the ETag header on PUT. The engine
        // must correct the recorded ETag from a follow-up PROPFIND, or every
        // later run would see "remote changed" and pull back its own push.
        let d = tmp_workdir("no-etag");
        let c = MockWebdavClient {
            put_returns_etag: false,
            ..Default::default()
        };
        text_file(&d.join("config.toml"), "cfg-v1");
        let files = vec![SyncFile {
            local_path: d.join("config.toml"),
            remote_name: "config.toml".into(),
            is_db: false,
        }];
        let mut state = SyncState::default();

        // First sync: empty remote → push all.
        let out = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap();
        assert_kind(&out.actions, "config.toml", "push");
        // The PUT carried no ETag; PROPFIND correction must fill it in.
        assert!(
            state.files["config.toml"].remote_etag.is_some(),
            "remote_etag must be corrected from PROPFIND after an ETag-less PUT"
        );

        // Second sync: nothing changed → skip (not pull!).
        let out2 = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap();
        assert_kind(&out2.actions, "config.toml", "skip");
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- db snapshot path ----

    #[tokio::test]
    async fn db_file_is_snapshotted_before_upload() {
        let d = tmp_workdir("db-snap");
        let c = MockWebdavClient::default();
        // Build a real sqlite file with one row.
        let db = d.join("note-personal.db");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (v TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES ('snap-row')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let mut state = SyncState::default();
        let files = vec![SyncFile {
            local_path: db.clone(),
            remote_name: "note-personal.db".into(),
            is_db: true,
        }];
        let actions = run_sync(&c, &files, &mut state, &opts(false, false), &resolve_none)
            .await
            .unwrap()
            .actions;
        assert_kind(&actions, "note-personal.db", "push");
        // The uploaded bytes must open as a sqlite db containing the row.
        let uploaded = c
            .files
            .lock()
            .unwrap()
            .get("note-personal.db")
            .unwrap()
            .clone();
        let tmp = d.join("uploaded-check.db");
        std::fs::write(&tmp, &uploaded).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&tmp))
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT v FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, "snap-row");
        pool.close().await;
        let _ = std::fs::remove_dir_all(&d);
    }
}
