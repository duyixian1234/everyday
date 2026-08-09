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

#[cfg(test)]
mod tests {
    use super::*;

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
}
