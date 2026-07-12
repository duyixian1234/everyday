//! Shared infrastructure for the local SQLite provider.
//!
//! The `local` (alias `sqlite`) providers of the `note` / `todo` / `bookmark`
//! modules reuse the connection setup, db-path resolution, and provider
//! discrimination logic defined here, so each module does not re-implement
//! them. See [R009](../../docs/adr/R009-notion-common-local-module.md).
//!
//! Design notes:
//! - Use [`sqlx`]'s `SqliteConnectOptions` (not a URL string) to avoid the
//!   Windows backslash escaping problem in `sqlite://` URLs.
//! - `create_if_missing(true)`: the file is created on demand, combined with
//!   each module's table creation to achieve "works on first use".
//! - Single-connection pool: each CLI invocation is a short-lived process, so
//!   one connection suffices and avoids SQLite write-concurrency locks.

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::error::{AgentError, Result};

/// Whether a provider string denotes the local SQLite provider.
///
/// Accepts both `local` and `sqlite` aliases (case-insensitive).
pub fn is_local_provider(provider: &str) -> bool {
    matches!(provider.to_ascii_lowercase().as_str(), "local" | "sqlite")
}

/// Resolve the local SQLite database file path.
///
/// - `override_path`: the account config's `db_path`; used directly if present.
/// - otherwise falls back to `~/.config/everyday/<module>-<account>.db`.
pub fn resolve_db_path(
    module: &str,
    account_name: &str,
    override_path: Option<&str>,
) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir
        .join("everyday")
        .join(format!("{module}-{account_name}.db")))
}

/// Open (creating if needed) the SQLite connection pool.
///
/// Creates the parent directory automatically; creates the db file if absent.
pub async fn connect(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

/// Parse a comma-separated tag string into a cleaned `Vec<String>`.
///
/// - trims leading/trailing whitespace per item;
/// - drops empty items (so `"rust, ,cli"` yields no empty tag);
/// - `None` input returns an empty `Vec`.
///
/// Previously `bookmark.rs` and `bookmark_local.rs` each had an identical
/// copy (`parse_tags` / `parse_tags_local_splits`); consolidated here. See
/// [R009](../../docs/adr/R009-notion-common-local-module.md).
pub fn parse_tags(raw: Option<&String>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    }
}

/// Find the account whose `name` matches in config's `<module>.accounts[]`
/// and write `default_database_id = db_id`.
///
/// Edits only that account's TOML entry; other accounts and sections are
/// untouched. Previously `todo.rs` and `bookmark.rs` each had a ~35-line
/// verbatim copy (`set_todo_database_id` / `set_bookmark_database_id`);
/// consolidated here. See [R009](../../docs/adr/R009-notion-common-local-module.md).
pub fn set_module_database_id(
    root: &mut toml::Value,
    module: &str,
    account_name: &str,
    db_id: &str,
) -> Result<()> {
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("config root is not a table".into()))?;
    let section = table
        .get_mut(module)
        .ok_or_else(|| AgentError::Config(format!("no [{module}] section in config")))?;
    let section_table = section
        .as_table_mut()
        .ok_or_else(|| AgentError::Config(format!("{module} is not a table")))?;
    let accounts = section_table
        .get_mut("accounts")
        .ok_or_else(|| AgentError::Config(format!("{module}.accounts missing")))?;
    let arr = accounts
        .as_array_mut()
        .ok_or_else(|| AgentError::Config(format!("{module}.accounts is not an array")))?;

    let mut found = false;
    for acc in arr.iter_mut() {
        if acc.get("name").and_then(|n| n.as_str()) == Some(account_name) {
            acc.as_table_mut()
                .ok_or_else(|| AgentError::Config(format!("{module} account is not a table")))?
                .insert(
                    "default_database_id".into(),
                    toml::Value::String(db_id.to_string()),
                );
            found = true;
            break;
        }
    }
    if !found {
        return Err(AgentError::Config(format!(
            "{module} account '{account_name}' not found in config"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_provider_accepts_aliases() {
        assert!(is_local_provider("local"));
        assert!(is_local_provider("sqlite"));
        assert!(is_local_provider("SQLite"));
        assert!(!is_local_provider("notion"));
    }

    #[test]
    fn resolve_db_path_prefers_override() {
        let p = resolve_db_path("note", "x", Some("/tmp/custom.db")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn resolve_db_path_default_contains_module_and_account() {
        let p = resolve_db_path("todo", "work", None).unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("todo-work.db"));
        assert!(s.contains("everyday"));
    }

    #[test]
    fn parse_tags_none_is_empty() {
        assert!(parse_tags(None).is_empty());
    }

    #[test]
    fn parse_tags_splits_trims_drops_empty() {
        let raw = "  rust , cli , ,  timeline  ".to_string();
        assert_eq!(parse_tags(Some(&raw)), vec!["rust", "cli", "timeline"]);
    }

    #[test]
    fn parse_tags_single_token() {
        let raw = "rust".to_string();
        assert_eq!(parse_tags(Some(&raw)), vec!["rust"]);
    }

    #[test]
    fn set_module_database_id_edits_only_target() {
        let mut root: toml::Value = toml::from_str(
            r#"
[default_account]
todo = "t1"
bookmark = "b1"

[[todo.accounts]]
name = "t1"
default_database_id = "old_t"

[[todo.accounts]]
name = "t2"

[[bookmark.accounts]]
name = "b1"
default_database_id = "old_b"
"#,
        )
        .unwrap();
        set_module_database_id(&mut root, "todo", "t1", "new_t").unwrap();

        // t1 should be updated; t2 / b1 / default_account untouched.
        let todo_accounts = root.get("todo").unwrap().get("accounts").unwrap();
        let t1 = todo_accounts
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some("t1"));
        assert_eq!(
            t1.unwrap().get("default_database_id").unwrap().as_str(),
            Some("new_t")
        );

        // b1 should be unchanged.
        let b1 = root
            .get("bookmark")
            .unwrap()
            .get("accounts")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some("b1"))
            .unwrap();
        assert_eq!(
            b1.get("default_database_id").unwrap().as_str(),
            Some("old_b")
        );
    }

    #[test]
    fn set_module_database_id_missing_account_errors() {
        let mut root: toml::Value = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
"#,
        )
        .unwrap();
        let err = set_module_database_id(&mut root, "todo", "ghost", "db").unwrap_err();
        assert!(err.message().contains("ghost"));
    }

    #[test]
    fn set_module_database_id_missing_module_errors() {
        let mut root: toml::Value = toml::Value::Table(toml::value::Table::new());
        let err = set_module_database_id(&mut root, "todo", "x", "db").unwrap_err();
        assert!(err.message().contains("todo"));
    }
}
