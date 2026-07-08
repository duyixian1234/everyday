//! 网络模块：网页抓取 + 通用 HTTP 工具。
//!
//! 骨架：实现 `Executor` 接口，reqwest/scraper 集成后续填充。

use async_trait::async_trait;

use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor};
use crate::output::Output;

pub struct NetworkModule;

impl NetworkModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Executor for NetworkModule {
    fn name(&self) -> &'static str { "net" }

    fn description(&self) -> &'static str {
        "Web fetching & HTTP tools: fetch URL → markdown, generic REST requests."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("fetch", "Fetch a URL and clean to markdown", "everyday net fetch <url>"),
            ActionDoc::new("request", "Generic HTTP request", "everyday net request --method POST --url URL [--body '...']"),
        ]
    }

    async fn execute(&self, action: &str, _args: &[String]) -> Result<Output> {
        match action {
            "fetch" | "request" => Err(AgentError::NotImplemented(format!(
                "net {action} — reqwest/scraper integration pending (see task_plan Phase 6)"
            ))),
            other => Err(AgentError::UnknownAction(format!("net {other}"))),
        }
    }
}
