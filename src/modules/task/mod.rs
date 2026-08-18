//! User-defined no-shell command execution and durable history (ADR F017).

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{Config, TaskConfig};
use crate::error::{AgentError, Result};
use crate::modules::{ActionArgSpec, Executor, HealthStatus, ModuleArgSpec, Positional};
use crate::output::{Output, TypedValue};
use crate::shared::request_context::RequestContext;
use crate::util::args::parse_simple_args;

pub mod runner;
pub mod scheduler;
pub mod store;

/// Task module backed by `[tasks]` config and `task.db`.
pub struct TaskModule {
    config: Arc<Config>,
    /// Lazily-opened task database, shared by every action and `health_check`
    /// so a resident process reuses one connection instead of re-opening per
    /// call. `OnceCell` (not a mutex): the module is never accessed
    /// concurrently — CLI runs are single-shot, MCP tools execute serially,
    /// and the daemon's scheduler owns a separate store instance.
    store: tokio::sync::OnceCell<store::TaskStore>,
}

impl TaskModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            store: tokio::sync::OnceCell::new(),
        }
    }

    /// The shared task store, opened once on first use.
    async fn task_store(&self) -> Result<&store::TaskStore> {
        self.store
            .get_or_try_init(store::TaskStore::open_default)
            .await
    }
}

#[async_trait]
impl Executor for TaskModule {
    fn description(&self) -> &'static str {
        "Run named no-shell commands, manage cron schedules, and query execution history."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "add",
                "新增命名任务（保留 config.toml 注释）",
                "everyday task add <name> --command <cmd> [--args <s>] [--allow-extra-args <bool>] [--timeout <secs>] [--capture-output <bool>] [--schedule <cron>]",
                &[
                    flag!("command", "可执行文件或路径（不经 shell）"),
                    flag!("args", "配置参数字符串（按空白拆分）", Value, Hyphen),
                    flag!("allow-extra-args", "是否允许 run 时追加参数（true/false）"),
                    flag!("timeout", "超时秒数；0 表示无限制"),
                    flag!(
                        "capture-output",
                        "手动执行时是否落库 stdout/stderr（true/false）"
                    ),
                    flag!("schedule", "标准 5 段 cron：min hour dom mon dow"),
                ],
                Positional::Exactly(1)
            ),
            cli_action!(
                "run",
                "执行命名任务；额外参数放在 -- 之后",
                "everyday task run <name> [-- extra...]",
                &[],
                Positional::OneOrMore
            ),
            cli_action!(
                "list",
                "列出全部任务配置",
                "everyday task list [--json]",
                &[]
            ),
            cli_action!(
                "remove",
                "删除任务配置（保留执行历史）",
                "everyday task remove <name>",
                &[],
                Positional::Exactly(1)
            ),
            cli_action!(
                "history",
                "查询任务执行历史",
                "everyday task history <name> [--limit N] [--json]",
                &[flag!("limit", "最大记录数；默认 20，0 表示无限制")],
                Positional::Exactly(1)
            ),
        ];
        ModuleArgSpec {
            name: "task",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String], ctx: &RequestContext) -> Result<Output> {
        match action {
            "add" => self.add(args),
            "run" => self.run(args, ctx).await,
            "list" => self.list(),
            "remove" => self.remove(args).await,
            "history" => self.history(args).await,
            other => Err(AgentError::UnknownAction(format!("task {other}"))),
        }
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        match self.task_store().await {
            Ok(_) => Ok(HealthStatus::healthy()),
            Err(error) => Ok(HealthStatus::degraded(format!(
                "task db: {}",
                error.message()
            ))),
        }
    }
}

impl TaskModule {
    fn add(&self, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let name = positional.first().ok_or_else(|| {
            AgentError::InvalidArgument("usage: everyday task add <name> --command <cmd>".into())
        })?;
        let command = flags.get("command").cloned().ok_or_else(|| {
            AgentError::InvalidArgument("task add requires --command <cmd>".into())
        })?;
        let task = TaskConfig {
            command,
            args: flags.get("args").cloned().unwrap_or_default(),
            allow_extra_args: parse_bool_flag(&flags, "allow-extra-args", false)?,
            timeout_secs: parse_u64_flag(&flags, "timeout", 60)?,
            capture_output: parse_bool_flag(&flags, "capture-output", false)?,
            schedule: flags
                .get("schedule")
                .cloned()
                .filter(|s| !s.trim().is_empty()),
        };
        validate_task(name, &task)?;
        crate::config::ConfigEditor::open()?.insert_task(name, &task)?;
        if crate::util::json_mode::is_json() {
            Ok(Output::Json(serde_json::json!({
                "name": name,
                "task": task,
            })))
        } else {
            Ok(Output::text(format!("added task `{name}`")))
        }
    }

    async fn run(&self, args: &[String], ctx: &RequestContext) -> Result<Output> {
        let name = args.first().ok_or_else(|| {
            AgentError::InvalidArgument("usage: everyday task run <name> [-- extra...]".into())
        })?;
        let task = self
            .config
            .tasks
            .get(name)
            .ok_or_else(|| AgentError::InvalidArgument(format!("task `{name}` not found")))?;
        let structured = crate::util::json_mode::is_json() || ctx.caller == "mcp";
        let relay = if structured {
            runner::RelayMode::Structured
        } else {
            runner::RelayMode::Terminal
        };
        let store = self.task_store().await?;
        let record = runner::run(store, name, task, &args[1..], relay).await?;
        // Mirror the child's exit status on the request context so the CLI
        // process exits with it; MCP ignores it (no process exit), reading
        // `status`/`exit_code` from `_result` instead ([R023]).
        ctx.set_exit_code(runner::mirrored_exit_code(&record));
        let output = if structured {
            Output::Json(serde_json::json!({ "_result": record }))
        } else {
            Output::text("")
        };
        Ok(output)
    }

    fn list(&self) -> Result<Output> {
        let mut tasks: Vec<(&String, &TaskConfig)> = self.config.tasks.iter().collect();
        tasks.sort_by(|a, b| a.0.cmp(b.0));
        if crate::util::json_mode::is_json() {
            let rows: Vec<serde_json::Value> = tasks
                .into_iter()
                .map(|(name, task)| {
                    serde_json::json!({
                        "name": name,
                        "command": task.command,
                        "args": task.args,
                        "allow_extra_args": task.allow_extra_args,
                        "timeout_secs": task.timeout_secs,
                        "capture_output": task.capture_output,
                        "schedule": task.schedule,
                    })
                })
                .collect();
            Ok(Output::Json(rows.into()))
        } else {
            let rows = tasks
                .into_iter()
                .map(|(name, task)| {
                    vec![
                        TypedValue::text(name),
                        TypedValue::text(&task.command),
                        TypedValue::text(&task.args),
                        TypedValue::boolean(task.allow_extra_args),
                        TypedValue::number(task.timeout_secs as f64),
                        TypedValue::boolean(task.capture_output),
                        task.schedule
                            .as_deref()
                            .map(TypedValue::text)
                            .unwrap_or_else(TypedValue::null),
                    ]
                })
                .collect();
            Ok(Output::typed_records(
                vec![
                    "name".into(),
                    "command".into(),
                    "args".into(),
                    "allow_extra_args".into(),
                    "timeout_secs".into(),
                    "capture_output".into(),
                    "schedule".into(),
                ],
                rows,
            ))
        }
    }

    async fn remove(&self, args: &[String]) -> Result<Output> {
        let (_flags, positional) = parse_simple_args(args);
        let name = positional.first().ok_or_else(|| {
            AgentError::InvalidArgument("usage: everyday task remove <name>".into())
        })?;
        if !self.config.tasks.contains_key(name) {
            return Err(AgentError::InvalidArgument(format!(
                "task `{name}` not found"
            )));
        }
        let store = self.task_store().await?;
        store.clear_schedule(name).await?;
        if !crate::config::ConfigEditor::open()?.remove_task(name)? {
            return Err(AgentError::InvalidArgument(format!(
                "task `{name}` not found"
            )));
        }
        if crate::util::json_mode::is_json() {
            Ok(Output::Json(serde_json::json!({
                "name": name,
                "removed": true,
                "history_retained": true,
            })))
        } else {
            Ok(Output::text(format!(
                "removed task `{name}` (history retained)"
            )))
        }
    }

    async fn history(&self, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let name = positional.first().ok_or_else(|| {
            AgentError::InvalidArgument("usage: everyday task history <name> [--limit N]".into())
        })?;
        let limit = parse_usize_flag(&flags, "limit", 20)?;
        let store = self.task_store().await?;
        let history = store.history(name, limit).await?;
        if crate::util::json_mode::is_json() {
            Ok(Output::Json(serde_json::to_value(history)?))
        } else {
            let rows = history
                .into_iter()
                .map(|run| {
                    vec![
                        run.id,
                        run.started_at,
                        run.status,
                        run.exit_code.map(|c| c.to_string()).unwrap_or_default(),
                        run.duration_ms.to_string(),
                    ]
                })
                .collect();
            Ok(Output::records(
                vec![
                    "id".into(),
                    "started_at".into(),
                    "status".into(),
                    "exit_code".into(),
                    "duration_ms".into(),
                ],
                rows,
            ))
        }
    }
}

/// Validate one `[tasks.<name>]` entry at config load and task creation.
pub fn validate_task(name: &str, task: &TaskConfig) -> Result<()> {
    crate::config::validate_task_config(name, task)
}

fn parse_bool_flag(
    flags: &std::collections::HashMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool> {
    match flags.get(name) {
        None => Ok(default),
        Some(value) => value
            .parse::<bool>()
            .map_err(|_| AgentError::InvalidArgument(format!("--{name} must be true or false"))),
    }
}

fn parse_u64_flag(
    flags: &std::collections::HashMap<String, String>,
    name: &str,
    default: u64,
) -> Result<u64> {
    match flags.get(name) {
        None => Ok(default),
        Some(value) => value.parse::<u64>().map_err(|_| {
            AgentError::InvalidArgument(format!("--{name} must be a non-negative integer"))
        }),
    }
}

fn parse_usize_flag(
    flags: &std::collections::HashMap<String, String>,
    name: &str,
    default: usize,
) -> Result<usize> {
    match flags.get(name) {
        None => Ok(default),
        Some(value) => value.parse::<usize>().map_err(|_| {
            AgentError::InvalidArgument(format!("--{name} must be a non-negative integer"))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(schedule: Option<&str>) -> TaskConfig {
        TaskConfig {
            command: "echo".into(),
            args: String::new(),
            allow_extra_args: false,
            timeout_secs: 60,
            capture_output: false,
            schedule: schedule.map(str::to_string),
        }
    }

    #[test]
    fn validates_name_command_and_cron() {
        assert!(validate_task("deploy-prod_2", &task(Some("*/5 * * * *"))).is_ok());
        assert!(validate_task("-bad", &task(None)).is_err());
        assert!(validate_task("bad space", &task(None)).is_err());
        assert!(validate_task("x", &task(Some("* * * *"))).is_err());
        let mut empty = task(None);
        empty.command = " ".into();
        assert!(validate_task("x", &empty).is_err());
    }

    #[test]
    fn default_timeout_and_bool_parsing() {
        let flags = std::collections::HashMap::new();
        assert_eq!(parse_u64_flag(&flags, "timeout", 60).unwrap(), 60);
        assert!(!parse_bool_flag(&flags, "capture-output", false).unwrap());
    }
}
