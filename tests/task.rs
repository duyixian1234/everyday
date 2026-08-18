//! Binary-level task module contracts (ADR F017).

#![cfg(not(target_os = "windows"))]

use assert_cmd::Command;
use std::path::{Path, PathBuf};

fn config_env_var() -> &'static str {
    if cfg!(target_os = "macos") {
        "HOME"
    } else {
        "XDG_CONFIG_HOME"
    }
}

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

fn temp_root(test: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("everyday-task-{}-{test}", std::process::id()))
}

fn write_config(root: &Path, content: &str) {
    let path = config_path_in(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn everyday_binary() -> String {
    assert_cmd::cargo::cargo_bin("everyday")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn add_preserves_comments_and_list_remove_are_structured() {
    let root = temp_root("config");
    let binary = everyday_binary();
    write_config(&root, "# user comment\n[daemon]\nenabled = true # keep\n");
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "add", "version", "--command"])
        .arg(&binary)
        .args(["--args=--version", "--capture-output", "true", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let config = std::fs::read_to_string(config_path_in(&root)).unwrap();
    assert!(config.contains("# user comment"));
    assert!(config.contains("enabled = true # keep"));
    assert!(config.contains("[tasks.version]"));

    let list = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "list", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(rows[0]["name"], "version");
    assert_eq!(rows[0]["capture_output"], true);

    let remove = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "remove", "version", "--json"])
        .output()
        .unwrap();
    assert_eq!(remove.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&remove.stdout).unwrap();
    assert_eq!(value["history_retained"], true);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn run_json_keeps_stdout_clean_and_history_is_durable() {
    let root = temp_root("run");
    write_config(
        &root,
        &format!(
            "[tasks.version]\ncommand = {}\nargs = \"--version\"\ncapture_output = true\n",
            toml::Value::String(everyday_binary())
        ),
    );
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "run", "version", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must contain only JSON");
    assert_eq!(value["_result"]["status"], "success");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("everyday"),
        "child version output must be redirected to stderr"
    );

    let history = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "history", "version", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(rows.as_array().map(Vec::len), Some(1));
    assert_eq!(rows[0]["status"], "success");
    assert!(
        rows[0]["stdout"]
            .as_str()
            .is_some_and(|s| s.contains("everyday"))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn run_mirrors_nonzero_child_exit() {
    let root = temp_root("exit");
    write_config(
        &root,
        &format!(
            "[tasks.bad]\ncommand = {}\nargs = \"not-a-command\"\n",
            toml::Value::String(everyday_binary())
        ),
    );
    let out = Command::cargo_bin("everyday")
        .unwrap()
        .env(config_env_var(), &root)
        .args(["task", "run", "bad", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["_result"]["status"], "failed");
    assert_eq!(value["_result"]["exit_code"], 2);
    let _ = std::fs::remove_dir_all(root);
}
