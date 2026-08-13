//! daemon module integration tests (v0.17.0, ADR F016, ticket t1).
//!
//! t1 wires the CLI surface: `[daemon]` config + `run`/`status` actions +
//! the `enabled` gate. These tests lock the binary-level behavior of the
//! gate: a disabled daemon must refuse to start with exit 1 and a clear
//! error on stdout (R001: command result is the stdout channel; the error
//! envelope follows the AgentError JSON shape, R002).
//!
//! Config isolation: `Config::load_or_default` reads
//! `dirs::config_dir()/everyday/config.toml`, so each test points the
//! subprocess at a private temp config dir via the platform-specific env
//! var `dirs` honors:
//! - Linux: `XDG_CONFIG_HOME` → `<temp>/everyday/config.toml`
//! - macOS: `HOME` → `<temp>/Library/Application Support/everyday/config.toml`
//!
//! Windows is excluded: `dirs::config_dir()` there uses the known-folder API
//! and ignores `APPDATA`, so the config path cannot be isolated per-subprocess
//! (verified 2026-08-13). On Windows the enabled gate is covered by the
//! module unit tests (`DaemonModule::execute` returns ConfigError when
//! disabled) + the cli_contract daemon command set.
//!
//! The env is set on the subprocess only (`Command::env`) — the test process
//! itself is never mutated, so parallel tests are unaffected.

#![cfg(not(target_os = "windows"))]

use assert_cmd::Command;
use std::path::{Path, PathBuf};

/// The env var `dirs::config_dir()` honors on this platform.
fn config_env_var() -> &'static str {
    if cfg!(target_os = "macos") {
        "HOME"
    } else {
        "XDG_CONFIG_HOME"
    }
}

/// Map a temp root to the `everyday/config.toml` path inside it, following
/// `dirs::config_dir()` per platform.
fn config_path_in(root: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        root.join("Library")
            .join("Application Support")
            .join("everyday")
            .join("config.toml")
    } else {
        root.join("everyday").join("config.toml")
    }
}

/// Unique temp root for one test (pid + test name avoids parallel collisions).
fn temp_root(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!("everyday-t1-{}-{test}", std::process::id()))
}

fn write_config(root: &Path, content: &str) {
    let path = config_path_in(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
}

#[test]
fn disabled_daemon_run_exits_1_with_clear_error() {
    let root = temp_root("disabled");
    write_config(&root, "[daemon]\nenabled = false\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("disabled"),
        "stdout should mention the disabled gate: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn disabled_daemon_run_json_error_shape() {
    let root = temp_root("json");
    write_config(&root, "[daemon]\nenabled = false\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // AgentError JSON envelope (R002): {"error": "...", "message": "..."}.
    assert!(
        stdout.contains("\"error\":\"ConfigError\"") && stdout.contains("disabled"),
        "JSON error envelope expected, got: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn disabled_daemon_run_ignores_once_flag() {
    // `--once` must not bypass the enabled gate.
    let root = temp_root("once");
    write_config(&root, "[daemon]\nenabled = false\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("disabled"),
        "stdout should mention the disabled gate: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_run_once_empty_config_succeeds() {
    // Empty config → one cycle runs (timeline opens the isolated cache db,
    // no accounts/feeds → zero counters), exit 0, summary on stdout.
    let root = temp_root("once-empty");
    write_config(&root, "# empty config\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("timeline:") && stdout.contains("mail:") && stdout.contains("rss:"),
        "one line per action expected: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_run_once_json_shape() {
    // `--once --json` → structured result object with the three actions.
    let root = temp_root("once-json");
    write_config(&root, "# empty config\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout must be JSON");
    for key in ["timeline", "mail", "rss"] {
        assert!(v.get(key).is_some(), "JSON missing `{key}`: {stdout}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ── t3: state file + status + anti-reentry ──

/// Path to the daemon state file inside an isolated config root.
fn state_path_in(root: &Path) -> PathBuf {
    config_path_in(root)
        .parent()
        .unwrap()
        .join("daemon-state.json")
}

/// Write a raw `daemon-state.json` into the isolated config root.
fn write_state(root: &Path, state: serde_json::Value) {
    let path = state_path_in(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, state.to_string()).unwrap();
}

#[test]
fn daemon_run_once_writes_complete_final_state() {
    // After `--once`, the state file must hold the exit snapshot: running=false,
    // exit_ok=true, cycles=1, and per-source results.
    let root = temp_root("state-after-once");
    write_config(&root, "# empty config\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let state_path = state_path_in(&root);
    assert!(state_path.exists(), "state file should exist after --once");
    let text = std::fs::read_to_string(&state_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).expect("state file must be JSON");
    assert_eq!(v["running"], serde_json::Value::Bool(false));
    assert_eq!(v["exit_ok"], serde_json::Value::Bool(true));
    assert_eq!(v["cycles"], serde_json::Value::from(1));
    for key in ["timeline", "mail", "rss"] {
        assert!(
            v["sources"][key].is_object(),
            "sources.{key} should be an object: {text}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_status_no_state_file_reports_not_running() {
    let root = temp_root("status-none");
    write_config(&root, "# empty config\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("not running") || stdout.contains("stopped"),
        "stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_status_after_once_shows_stopped_with_cycle_info() {
    let root = temp_root("status-after-once");
    write_config(&root, "# empty config\n");
    Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Status: stopped"), "stdout: {stdout}");
    assert!(stdout.contains("Cycles: 1"), "stdout: {stdout}");
    assert!(stdout.contains("timeline:"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_status_shows_running_for_live_pid() {
    // State claims running=true and the pid is a live child → running.
    let root = temp_root("status-live");
    write_config(&root, "# empty config\n");
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    write_state(
        &root,
        serde_json::json!({
            "pid": child.id(), "running": true, "enabled": true, "interval_seconds": 900,
            "started_at": "2026-08-13T23:00:00Z", "last_cycle_at": null,
            "cycles": 1, "last_cycle_ok": true, "exit_at": null, "exit_ok": null,
            "sources": {"timeline": null, "mail": null, "rss": null}
        }),
    );
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Status: running"), "stdout: {stdout}");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_status_shows_stopped_for_dead_pid() {
    // State claims running=true but the pid is gone → stopped (stale state).
    let root = temp_root("status-dead");
    write_config(&root, "# empty config\n");
    write_state(
        &root,
        serde_json::json!({
            "pid": 9_999_999, "running": true, "enabled": true, "interval_seconds": 900,
            "started_at": "2026-08-13T23:00:00Z", "last_cycle_at": null,
            "cycles": 1, "last_cycle_ok": true, "exit_at": null, "exit_ok": null,
            "sources": {"timeline": null, "mail": null, "rss": null}
        }),
    );
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Status: stopped"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_status_json_shape() {
    // `status --json` must emit the state object (R001: command result), with
    // the full ADR F016 schema, parseable as JSON.
    let root = temp_root("status-json");
    write_config(&root, "# empty config\n");
    Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "status", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("status --json must be JSON");
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
        assert!(v.get(key).is_some(), "missing {key}: {stdout}");
    }
    assert_eq!(v["running"], serde_json::Value::Bool(false));
    for key in ["timeline", "mail", "rss"] {
        assert!(v["sources"][key].is_object(), "sources.{key}: {stdout}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn daemon_run_refuses_when_another_instance_live() {
    // A state file claiming a live pid → `run` must exit 1 with a clear error
    // (anti-reentry, t3).
    let root = temp_root("reentry");
    write_config(&root, "# empty config\n");
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    write_state(
        &root,
        serde_json::json!({
            "pid": child.id(), "running": true, "enabled": true, "interval_seconds": 900,
            "started_at": "2026-08-13T23:00:00Z", "last_cycle_at": null,
            "cycles": 0, "last_cycle_ok": null, "exit_at": null, "exit_ok": null,
            "sources": {"timeline": null, "mail": null, "rss": null}
        }),
    );
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["daemon", "run", "--once"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("already running"), "stdout: {stdout}");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
}
