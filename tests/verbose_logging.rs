//! Binary-level tests for the leveled logging contract (default quiet,
//! `-v` restores middleware progress lines). Uses `config path` — a pure
//! local command with no network or credential access.

use assert_cmd::Command;

#[test]
fn default_stderr_has_no_middleware_progress_lines() {
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .args(["config", "path"])
        .output()
        .unwrap();
    assert!(out.status.success(), "`everyday config path` must exit 0");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        !stderr.contains(" start") && !stderr.contains("ok in") && !stderr.contains("error in"),
        "default (WARN) must silence middleware progress lines; stderr was:\n{stderr}"
    );
}

#[test]
fn verbose_restores_middleware_progress_lines() {
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .args(["-v", "config", "path"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "`everyday -v config path` must exit 0"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(" config path start"),
        "`-v` must show the start line; stderr was:\n{stderr}"
    );
    assert!(
        stderr.contains("ok in"),
        "`-v` must show the ok/duration line; stderr was:\n{stderr}"
    );
}

#[test]
fn json_mode_logs_are_structured() {
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .args(["-v", "--json", "config", "path"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    // Middleware lines in JSON mode must be `{"_log": ...}` structured lines.
    let start_line = stderr
        .lines()
        .find(|l| l.contains("\"_log\":\"start\""))
        .unwrap_or_else(|| panic!("missing `_log: start` line; stderr was:\n{stderr}"));
    let v: serde_json::Value = serde_json::from_str(start_line).unwrap();
    assert_eq!(v["_log"], "start");
    assert_eq!(v["module"], "config");
    assert_eq!(v["action"], "path");
}
