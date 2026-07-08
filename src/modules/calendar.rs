//! 日历模块（CalDAV）。
//!
//! 骨架：实现 `Executor` 接口，CalDAV 客户端逻辑后续填充。

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{parse_simple_args, ActionDoc, Executor};
use crate::output::Output;

pub struct CalendarModule {
    #[allow(dead_code)]
    config: Arc<Config>,
}

impl CalendarModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for CalendarModule {
    fn name(&self) -> &'static str { "cal" }

    fn description(&self) -> &'static str {
        "Calendar management (CalDAV): list, add, update, delete events."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("list", "List events", "everyday cal list [--today|--date YYYY-MM-DD] [--account NAME]"),
            ActionDoc::new("add", "Add an event", "everyday cal add --title T --start ISO --end ISO [--account NAME]"),
            ActionDoc::new("delete", "Delete an event", "everyday cal delete --id ID [--account NAME]"),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _) = parse_simple_args(args);
        let _account = self.config.calendar_account(flags.get("account").map(|s| s.as_str()))?;

        match action {
            "list" | "add" | "delete" => Err(AgentError::NotImplemented(format!(
                "cal {action} — CalDAV integration pending (see task_plan Phase 6)"
            ))),
            other => Err(AgentError::UnknownAction(format!("cal {other}"))),
        }
    }
}
