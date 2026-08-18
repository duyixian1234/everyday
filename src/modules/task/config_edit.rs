//! Comment-preserving task configuration edits via `toml_edit`.

use std::path::Path;
use std::str::FromStr;

use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::TaskConfig;
use crate::error::{AgentError, Result};

/// Add a task to the canonical config file.
pub fn add_task(name: &str, task: &TaskConfig) -> Result<()> {
    add_task_at(&crate::config::Config::config_path()?, name, task)
}

/// Remove a task from the canonical config file.
pub fn remove_task(name: &str) -> Result<bool> {
    remove_task_at(&crate::config::Config::config_path()?, name)
}

fn load_document(path: &Path) -> Result<DocumentMut> {
    let text = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };
    DocumentMut::from_str(&text)
        .map_err(|e| AgentError::Config(format!("failed to parse config for task edit: {e}")))
}

fn save_document(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

pub(crate) fn add_task_at(path: &Path, name: &str, task: &TaskConfig) -> Result<()> {
    let mut doc = load_document(path)?;
    if !doc.as_table().contains_key("tasks") {
        doc["tasks"] = Item::Table(Table::new());
    }
    let tasks = doc["tasks"]
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("`tasks` must be a TOML table".into()))?;
    if tasks.contains_key(name) {
        return Err(AgentError::InvalidArgument(format!(
            "task `{name}` already exists"
        )));
    }

    let mut table = Table::new();
    table.set_implicit(false);
    table["command"] = value(&task.command);
    if !task.args.is_empty() {
        table["args"] = value(&task.args);
    }
    table["allow_extra_args"] = value(task.allow_extra_args);
    table["timeout_secs"] = value(i64::try_from(task.timeout_secs).unwrap_or(i64::MAX));
    table["capture_output"] = value(task.capture_output);
    if let Some(schedule) = task.schedule.as_deref()
        && !schedule.trim().is_empty()
    {
        table["schedule"] = value(schedule);
    }
    tasks.insert(name, Item::Table(table));
    save_document(path, &doc)
}

pub(crate) fn remove_task_at(path: &Path, name: &str) -> Result<bool> {
    let mut doc = load_document(path)?;
    let Some(tasks) = doc.get_mut("tasks").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let removed = tasks.remove(name).is_some();
    if removed {
        save_document(path, &doc)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-config-{}-{name}.toml", std::process::id()))
    }

    #[test]
    fn add_preserves_comments_and_remove_preserves_history_independence() {
        let path = path("comments");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# keep me\n[daemon]\nenabled = true # inline\n").unwrap();
        let task = TaskConfig {
            command: "echo".into(),
            args: "hello".into(),
            allow_extra_args: false,
            timeout_secs: 10,
            capture_output: true,
            schedule: Some("*/5 * * * *".into()),
        };
        add_task_at(&path, "hello", &task).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"));
        assert!(text.contains("enabled = true # inline"));
        assert!(text.contains("[tasks.hello]"));
        assert!(remove_task_at(&path, "hello").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"));
        assert!(!text.contains("[tasks.hello]"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn duplicate_is_rejected() {
        let path = path("duplicate");
        std::fs::write(&path, "[tasks.x]\ncommand = \"echo\"\n").unwrap();
        let task = TaskConfig {
            command: "echo".into(),
            args: String::new(),
            allow_extra_args: false,
            timeout_secs: 60,
            capture_output: false,
            schedule: None,
        };
        assert!(add_task_at(&path, "x", &task).is_err());
        let _ = std::fs::remove_file(path);
    }
}
