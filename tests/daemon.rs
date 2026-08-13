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
