//! 邮件模块（IMAP 收 / SMTP 发）。
//!
//! 当前为骨架：实现 `Executor` 接口与动作文档，
//! 实际 IMAP/SMTP 逻辑在后续阶段填充。

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{parse_simple_args, ActionDoc, Executor};
use crate::output::Output;

pub struct EmailModule {
    #[allow(dead_code)]
    config: Arc<Config>,
}

impl EmailModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for EmailModule {
    fn name(&self) -> &'static str { "mail" }

    fn description(&self) -> &'static str {
        "Email management (IMAP/SMTP): list, send, search, attachments."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("list", "List messages", "everyday mail list [--unread] [--limit N] [--account NAME]"),
            ActionDoc::new("send", "Send a message", "everyday mail send --to ADDR --subject S --body TEXT [--account NAME]"),
            ActionDoc::new("search", "Search messages", "everyday mail search --query Q [--account NAME]"),
            ActionDoc::new("read", "Read a single message", "everyday mail read --id ID [--account NAME]"),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        // 验证账户配置可解析（提前失败，给出清晰错误）。
        let _account = self.config.mail_account(flags.get("account").map(|s| s.as_str()))?;

        match action {
            "list" | "send" | "search" | "read" => Err(AgentError::NotImplemented(format!(
                "mail {action} — IMAP/SMTP integration pending (see task_plan Phase 6)"
            ))),
            other => Err(AgentError::UnknownAction(format!("mail {other}"))),
        }
    }
}
