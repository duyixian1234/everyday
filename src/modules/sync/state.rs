//! Local sync state (`sync-state.json`) — the single source of truth for
//! change detection (ADR D002).
//!
//! Records, per synced file: the local content hash, the remote ETag, and the
//! remote Last-Modified. The file is **not** synced itself; deleting it makes
//! the next sync treat every file as changed (`--force` is equivalent).
//!
//! `sync-state.json` lives next to the everyday config/data directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};

/// Per-file sync state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    /// SHA-256 of the last locally-synced snapshot.
    #[serde(default)]
    pub local_hash: Option<String>,
    /// Remote ETag from the last upload / download.
    #[serde(default)]
    pub remote_etag: Option<String>,
    /// Remote Last-Modified (display / LWW arbitration).
    #[serde(default)]
    pub remote_mtime: Option<String>,
}

/// The whole sync-state document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Remote directory URL this state belongs to (guards against pointing
    /// two accounts at the same state file).
    #[serde(default)]
    pub remote_url: String,
    /// File name → state.
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
}

/// Where the state file lives: `<config_dir>/everyday/sync-state.json`.
pub fn state_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("sync-state.json"))
}

/// Load the state file. Missing file → empty state; corrupted file → error
/// surfaced to the user (with a `--force` hint) rather than silent reset,
/// which could re-upload everything without the user knowing why.
pub fn load(path: &Path) -> Result<SyncState> {
    if !path.exists() {
        return Ok(SyncState::default());
    }
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(SyncState::default());
    }
    serde_json::from_str(&text).map_err(|e| {
        AgentError::Other(format!(
            "sync-state.json corrupt: {e} (run `everyday sync --force` to rebuild)"
        ))
    })
}

/// Atomically save the state (write temp + rename, mirroring D002's atomic
/// replacement discipline).
pub fn save(path: &Path, state: &SyncState) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(state)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("everyday-state-{name}-{}", std::process::id()))
    }

    #[test]
    fn missing_file_yields_default() {
        let p = tmp_path("missing");
        let _ = std::fs::remove_file(&p);
        let s = load(&p).unwrap();
        assert!(s.files.is_empty());
        assert!(s.remote_url.is_empty());
    }

    #[test]
    fn roundtrip_preserves_state() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        let mut s = SyncState {
            remote_url: "https://dav.example.com/everyday".into(),
            ..Default::default()
        };
        s.files.insert(
            "memory.db".into(),
            FileState {
                local_hash: Some("abc".into()),
                remote_etag: Some("\"xyz\"".into()),
                remote_mtime: Some("Mon, 09 Aug 2026 12:00:00 GMT".into()),
            },
        );
        save(&p, &s).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!(loaded, s);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn empty_file_yields_default() {
        let p = tmp_path("empty");
        std::fs::write(&p, "  \n").unwrap();
        let s = load(&p).unwrap();
        assert!(s.files.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_errors_with_force_hint() {
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{ not json").unwrap();
        let err = load(&p).unwrap_err();
        assert_eq!(err.type_name(), "Other");
        assert!(err.message().contains("--force"), "{}", err.message());
        let _ = std::fs::remove_file(&p);
    }
}
