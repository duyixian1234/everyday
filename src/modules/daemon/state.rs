//! Daemon state file (`daemon-state.json`) — the running snapshot read by
//! `daemon status` and guarded against re-entry by `daemon run` (ADR F016, t3).
//!
//! The state is written at three moments:
//! 1. **startup** — pid / running=true / started_at
//! 2. **per-cycle** — last_cycle_at / cycles / last_cycle_ok / sources
//! 3. **exit** — running=false / exit_at / exit_ok (sources preserved)
//!
//! Writes are atomic (temp file + rename). A write failure logs a `warn!`
//! but never aborts the cycle — the daemon is best-effort (L009).
//!
//! PID liveness is probed cross-platform with zero extra dependencies:
//! Linux `/proc/<pid>`, macOS `kill -0`, Windows `tasklist /FI`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};
use crate::modules::daemon::cycle::ActionResult;

// ─── Schema ───────────────────────────────────────────────────────────

/// Timeline source result in the state file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineSourceState {
    pub ok: bool,
    pub events: usize,
    pub error: Option<String>,
}

/// Mail source result in the state file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MailSourceState {
    pub ok: bool,
    pub folders: usize,
    pub envelopes: usize,
    pub error: Option<String>,
}

/// RSS source result in the state file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RssSourceState {
    pub ok: bool,
    pub items: usize,
    pub error: Option<String>,
}

/// Per-source results (all `None` until the first cycle completes).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DaemonSources {
    pub timeline: Option<TimelineSourceState>,
    pub mail: Option<MailSourceState>,
    pub rss: Option<RssSourceState>,
}

/// The complete daemon state document (serialised to `daemon-state.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DaemonState {
    pub pid: u32,
    pub running: bool,
    pub enabled: bool,
    pub interval_seconds: u64,
    pub started_at: Option<DateTime<Utc>>,
    pub last_cycle_at: Option<DateTime<Utc>>,
    pub cycles: u64,
    pub last_cycle_ok: Option<bool>,
    pub exit_at: Option<DateTime<Utc>>,
    pub exit_ok: Option<bool>,
    pub sources: DaemonSources,
}

impl DaemonState {
    /// Initial state written at daemon startup.
    pub fn initial(pid: u32, enabled: bool, interval_seconds: u64) -> Self {
        Self {
            pid,
            running: true,
            enabled,
            interval_seconds,
            started_at: Some(Utc::now()),
            last_cycle_at: None,
            cycles: 0,
            last_cycle_ok: None,
            exit_at: None,
            exit_ok: None,
            sources: DaemonSources::default(),
        }
    }

    /// Update per-cycle fields after a sync cycle completes.
    pub fn update_cycle(&mut self, sources: DaemonSources) {
        self.last_cycle_at = Some(Utc::now());
        self.cycles += 1;
        // ok = all executed actions succeeded (skipped sources don't count).
        self.last_cycle_ok = Some(
            sources.timeline.as_ref().map(|s| s.ok).unwrap_or(true)
                && sources.mail.as_ref().map(|s| s.ok).unwrap_or(true)
                && sources.rss.as_ref().map(|s| s.ok).unwrap_or(true),
        );
        self.sources = sources;
    }

    /// Mark the daemon as stopped (exit state).
    pub fn mark_exit(&mut self, ok: bool) {
        self.running = false;
        self.exit_at = Some(Utc::now());
        self.exit_ok = Some(ok);
    }

    /// Effective running state: `running` flag AND pid still alive.
    pub fn is_effectively_running(&self) -> bool {
        self.running && pid_alive(self.pid)
    }
}

impl DaemonSources {
    /// Build from the cycle engine's `ActionResult` options.
    pub fn from_cycle(
        timeline: &Option<ActionResult>,
        mail: &Option<ActionResult>,
        rss: &Option<ActionResult>,
    ) -> Self {
        Self {
            timeline: timeline.as_ref().map(|a| TimelineSourceState {
                ok: a.ok,
                events: a.events,
                error: a.error.clone(),
            }),
            mail: mail.as_ref().map(|a| MailSourceState {
                ok: a.ok,
                folders: a.folders,
                envelopes: a.envelopes,
                error: a.error.clone(),
            }),
            rss: rss.as_ref().map(|a| RssSourceState {
                ok: a.ok,
                items: a.items,
                error: a.error.clone(),
            }),
        }
    }
}

// ─── Path ─────────────────────────────────────────────────────────────

/// The everyday config dir: `<config_dir>/everyday`.
fn config_everyday_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday"))
}

/// Where the state file lives: `<config_dir>/everyday/daemon-state.json`.
pub fn state_path() -> Result<PathBuf> {
    Ok(config_everyday_dir()?.join("daemon-state.json"))
}

/// Where the daemon file log lives: `<config_dir>/everyday/daemon.log`
/// (ADR [F016](../../docs/adr/F016-daemon-sync-scheduler.md) t4 — fixed
/// INFO, append, no rotation).
pub fn daemon_log_path() -> Result<PathBuf> {
    Ok(config_everyday_dir()?.join("daemon.log"))
}

// ─── Read / Write ─────────────────────────────────────────────────────

/// Read the state file. Returns `None` when the file is missing (daemon never
/// started or state was deleted).
pub fn read() -> Result<Option<DaemonState>> {
    let path = state_path()?;
    read_from(&path)
}

/// Read state from a specific path (test-friendly).
pub fn read_from(path: &Path) -> Result<Option<DaemonState>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| AgentError::Other(format!("daemon-state.json corrupt: {e}")))
}

/// Atomic write that **propagates** errors — used by the exit path (t4): a
/// failed final state write must surface as `_error` + exit 1, unlike the
/// best-effort startup / per-cycle writes.
pub fn write_result(state: &DaemonState) -> Result<()> {
    let path = state_path()?;
    write_to(&path, state)
}

/// Best-effort atomic write (temp + rename). Failures are logged as `warn!`
/// but do not propagate — the daemon is best-effort (L009).
pub fn write(state: &DaemonState) {
    if let Err(e) = write_result(state) {
        tracing::warn!(
            target: "everyday",
            error = %e.message(),
            "daemon: state write failed"
        );
    }
}

/// Atomic write to a specific path: serialise → write temp → rename.
/// Mirrors D002 / `sync::state::save`.
pub fn write_to(path: &Path, state: &DaemonState) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    // Temp file name with nanosecond suffix to avoid concurrent collision
    // (pattern from sync/engine.rs `replace_local`).
    let tmp = path.with_extension(format!(
        "tmp{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let text = serde_json::to_string_pretty(state)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ─── PID liveness ─────────────────────────────────────────────────────

/// Check whether a process with the given PID is alive.
///
/// Cross-platform, zero extra dependencies:
/// - Linux: `/proc/<pid>` existence (fastest, no subprocess)
/// - macOS / other Unix: `kill -0 <pid>` (standard signal-less probe)
/// - Windows: `tasklist /FI "PID eq <pid>" /NH /FO CSV` — the no-match
///   message is **localized** ("INFO: ..." / "信息: ..."), so we never match
///   on its text; instead we look for the PID as a quoted CSV field, which
///   only a real process row contains.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // A matching row looks like `"everyday.exe","12345",...`; the
                // localized no-match / invalid-query messages never contain a
                // quoted PID, so this is locale-independent.
                stdout.contains(&format!("\"{pid}\""))
            })
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

// ─── Anti-reentry ─────────────────────────────────────────────────────

/// Check whether another daemon instance is already running. Returns `Ok(())`
/// when it is safe to start, or an error with the live PID.
pub fn check_reentry() -> Result<()> {
    let path = state_path()?;
    check_reentry_at(&path)
}

/// Check reentry against a specific state file path (test-friendly).
pub fn check_reentry_at(path: &Path) -> Result<()> {
    let existing = read_from(path)?;
    if let Some(state) = existing
        && state.running
        && pid_alive(state.pid)
    {
        return Err(AgentError::Other(format!(
            "daemon already running (pid {}); remove daemon-state.json if stale",
            state.pid
        )));
    }
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::daemon::cycle::ActionResult;

    /// Unique temp path per test (includes PID to avoid parallel collision).
    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "everyday-daemon-state-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    // ── Schema / field correctness ──

    #[test]
    fn initial_state_fields() {
        let s = DaemonState::initial(42, true, 900);
        assert_eq!(s.pid, 42);
        assert!(s.running);
        assert!(s.enabled);
        assert_eq!(s.interval_seconds, 900);
        assert!(s.started_at.is_some());
        assert!(s.last_cycle_at.is_none());
        assert_eq!(s.cycles, 0);
        assert!(s.last_cycle_ok.is_none());
        assert!(s.exit_at.is_none());
        assert!(s.exit_ok.is_none());
        assert!(s.sources.timeline.is_none());
        assert!(s.sources.mail.is_none());
        assert!(s.sources.rss.is_none());
    }

    #[test]
    fn update_cycle_sets_fields() {
        let mut s = DaemonState::initial(1, true, 60);
        let sources = DaemonSources {
            timeline: Some(TimelineSourceState {
                ok: true,
                events: 5,
                error: None,
            }),
            mail: Some(MailSourceState {
                ok: true,
                folders: 3,
                envelopes: 10,
                error: None,
            }),
            rss: None,
        };
        s.update_cycle(sources);
        assert_eq!(s.cycles, 1);
        assert!(s.last_cycle_at.is_some());
        assert_eq!(s.last_cycle_ok, Some(true));
        assert_eq!(s.sources.timeline.as_ref().unwrap().events, 5);
        assert_eq!(s.sources.mail.as_ref().unwrap().folders, 3);
        assert!(s.sources.rss.is_none());
    }

    #[test]
    fn update_cycle_ok_false_when_any_source_fails() {
        let mut s = DaemonState::initial(1, true, 60);
        let sources = DaemonSources {
            timeline: Some(TimelineSourceState {
                ok: true,
                events: 5,
                error: None,
            }),
            mail: Some(MailSourceState {
                ok: false,
                folders: 0,
                envelopes: 0,
                error: Some("connection refused".into()),
            }),
            rss: None,
        };
        s.update_cycle(sources);
        assert_eq!(s.last_cycle_ok, Some(false));
    }

    #[test]
    fn update_cycle_ok_true_when_all_skipped() {
        // No sources executed — ok defaults to true (nothing failed).
        let mut s = DaemonState::initial(1, true, 60);
        s.update_cycle(DaemonSources::default());
        assert_eq!(s.last_cycle_ok, Some(true));
    }

    #[test]
    fn mark_exit_sets_fields() {
        let mut s = DaemonState::initial(1, true, 60);
        s.mark_exit(true);
        assert!(!s.running);
        assert!(s.exit_at.is_some());
        assert_eq!(s.exit_ok, Some(true));
    }

    // ── Atomic write / read roundtrip ──

    #[test]
    fn write_then_read_roundtrip() {
        let path = tmp_path("roundtrip");
        let state = DaemonState::initial(99, true, 300);
        write_to(&path, &state).unwrap();
        let loaded = read_from(&path).unwrap().unwrap();
        assert_eq!(loaded, state);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_missing_file_returns_none() {
        let path = tmp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert!(read_from(&path).unwrap().is_none());
    }

    #[test]
    fn read_empty_file_returns_none() {
        let path = tmp_path("empty");
        std::fs::write(&path, "  \n").unwrap();
        assert!(read_from(&path).unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_corrupt_file_errors() {
        let path = tmp_path("corrupt");
        std::fs::write(&path, "{ not json").unwrap();
        let err = read_from(&path).unwrap_err();
        assert_eq!(err.type_name(), "Other");
        assert!(err.message().contains("corrupt"), "{}", err.message());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!(
            "everyday-daemon-test-parent-{}",
            std::process::id()
        ));
        let path = dir.join("sub").join("daemon-state.json");
        let _ = std::fs::remove_dir_all(&dir);
        let state = DaemonState::initial(1, true, 60);
        write_to(&path, &state).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_does_not_leave_temp_on_success() {
        let path = tmp_path("no_temp");
        let state = DaemonState::initial(1, true, 60);
        write_to(&path, &state).unwrap();
        // The temp file should have been renamed away. `with_extension` turns
        // `daemon-state.json` into `daemon-state.tmp<nanos>`, so the leftover
        // prefix to scan for is "daemon-state.tmp".
        let entries = std::fs::read_dir(path.parent().unwrap()).unwrap();
        let temps: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("daemon-state.tmp")
            })
            .collect();
        assert!(temps.is_empty(), "leftover temp files: {temps:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_failure_keeps_destination_intact() {
        // Simulate a failure at the rename step: the destination is a
        // directory, so `rename(tmp, path)` fails. The destination must stay
        // untouched (never half-written) — the atomicity contract.
        let dir = tmp_path("rename_fail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon-state.json");
        std::fs::create_dir(&path).unwrap(); // destination is a dir → rename fails
        let state = DaemonState::initial(1, true, 60);
        assert!(
            write_to(&path, &state).is_err(),
            "rename onto an existing directory must fail"
        );
        assert!(path.is_dir(), "destination must be untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── from_cycle conversion ──

    #[test]
    fn from_cycle_converts_all_sources() {
        let timeline = Some(ActionResult::timeline_ok(7));
        let mail = Some(ActionResult::mail_ok(4, 15));
        let rss = Some(ActionResult::rss_ok(3));
        let sources = DaemonSources::from_cycle(&timeline, &mail, &rss);
        assert_eq!(sources.timeline.as_ref().unwrap().events, 7);
        let mail_state = sources.mail.as_ref().unwrap();
        assert_eq!(mail_state.folders, 4);
        assert_eq!(mail_state.envelopes, 15);
        assert_eq!(sources.rss.as_ref().unwrap().items, 3);
    }

    #[test]
    fn from_cycle_with_failed_action() {
        let timeline = Some(ActionResult::failed("timeout"));
        let sources = DaemonSources::from_cycle(&timeline, &None, &None);
        let tl = sources.timeline.unwrap();
        assert!(!tl.ok);
        assert_eq!(tl.error.as_deref(), Some("timeout"));
        assert!(sources.mail.is_none());
        assert!(sources.rss.is_none());
    }

    #[test]
    fn from_cycle_with_all_none() {
        let sources = DaemonSources::from_cycle(&None, &None, &None);
        assert!(sources.timeline.is_none());
        assert!(sources.mail.is_none());
        assert!(sources.rss.is_none());
    }

    // ── PID liveness ──

    #[test]
    fn pid_alive_current_process() {
        // The test runner itself is alive.
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn pid_alive_nonexistent_pid() {
        // A normal-range PID that no real process will hold. (Avoid 0xFFFFFFFC:
        // on Windows tasklist answers "invalid query" for out-of-range PIDs,
        // which would not exercise the real no-match path.)
        assert!(!pid_alive(99_999_999));
    }

    // ── Anti-reentry ──

    #[test]
    fn check_reentry_passes_when_no_file() {
        let path = tmp_path("reentry_none");
        let _ = std::fs::remove_file(&path);
        assert!(check_reentry_at(&path).is_ok());
    }

    #[test]
    fn check_reentry_passes_when_stopped() {
        let path = tmp_path("reentry_stopped");
        let mut state = DaemonState::initial(1, true, 60);
        state.mark_exit(true);
        write_to(&path, &state).unwrap();
        assert!(check_reentry_at(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn check_reentry_passes_when_running_but_pid_dead() {
        let path = tmp_path("reentry_dead");
        let state = DaemonState::initial(99_999_999, true, 60);
        write_to(&path, &state).unwrap();
        // The pid is not alive, so reentry should pass.
        assert!(check_reentry_at(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn check_reentry_fails_when_running_and_pid_alive() {
        let path = tmp_path("reentry_alive");
        let state = DaemonState::initial(std::process::id(), true, 60);
        write_to(&path, &state).unwrap();
        let err = check_reentry_at(&path).unwrap_err();
        assert_eq!(err.type_name(), "Other");
        assert!(
            err.message().contains("already running"),
            "{}",
            err.message()
        );
        let _ = std::fs::remove_file(&path);
    }

    // ── Serialization shape ──

    #[test]
    fn serde_roundtrip_preserves_all_fields() {
        let state = DaemonState {
            pid: 12345,
            running: false,
            enabled: true,
            interval_seconds: 900,
            started_at: Some(
                DateTime::parse_from_rfc3339("2026-08-13T23:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            last_cycle_at: Some(
                DateTime::parse_from_rfc3339("2026-08-13T23:15:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            cycles: 3,
            last_cycle_ok: Some(true),
            exit_at: Some(
                DateTime::parse_from_rfc3339("2026-08-13T23:20:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            exit_ok: Some(true),
            sources: DaemonSources {
                timeline: Some(TimelineSourceState {
                    ok: true,
                    events: 12,
                    error: None,
                }),
                mail: Some(MailSourceState {
                    ok: true,
                    folders: 8,
                    envelopes: 34,
                    error: None,
                }),
                rss: Some(RssSourceState {
                    ok: true,
                    items: 5,
                    error: None,
                }),
            },
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: DaemonState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn json_output_has_expected_shape() {
        let state = DaemonState::initial(42, true, 900);
        let json = serde_json::to_value(&state).unwrap();
        // Verify top-level keys match the ADR F016 schema.
        for key in [
            "pid",
            "running",
            "enabled",
            "interval_seconds",
            "started_at",
            "last_cycle_at",
            "cycles",
            "last_cycle_ok",
            "exit_at",
            "exit_ok",
            "sources",
        ] {
            assert!(json.get(key).is_some(), "missing key: {key}");
        }
        // sources sub-keys
        let sources = json.get("sources").unwrap();
        for key in ["timeline", "mail", "rss"] {
            assert!(sources.get(key).is_some(), "missing sources.{key}");
        }
    }
}
