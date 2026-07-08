//! RSS/Atom 订阅模块。
//!
//! 骨架：实现 `Executor` 接口，订阅源管理与摘要聚合后续填充。

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor};
use crate::output::Output;

pub struct RssModule {
    #[allow(dead_code)]
    config: Arc<Config>,
}

impl RssModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for RssModule {
    fn name(&self) -> &'static str { "rss" }

    fn description(&self) -> &'static str {
        "RSS/Atom feed management: follow, list, digest."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("follow", "Add a feed", "everyday rss follow --name N --url URL [--category C]"),
            ActionDoc::new("list", "List followed feeds", "everyday rss list"),
            ActionDoc::new("digest", "Aggregate recent entries", "everyday rss digest [--limit N]"),
        ]
    }

    async fn execute(&self, action: &str, _args: &[String]) -> Result<Output> {
        match action {
            "follow" | "list" | "digest" => Err(AgentError::NotImplemented(format!(
                "rss {action} — feed-rs integration pending (see task_plan Phase 6)"
            ))),
            other => Err(AgentError::UnknownAction(format!("rss {other}"))),
        }
    }
}
