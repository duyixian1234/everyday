//! 文件系统模块：文件搜索、目录树、结构化读取。
//!
//! 骨架：实现 `Executor` 接口，ignore/walkdir/serde 集成后续填充。

use async_trait::async_trait;

use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor};
use crate::output::Output;

pub struct FsModule;

impl FsModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Executor for FsModule {
    fn name(&self) -> &'static str { "fs" }

    fn description(&self) -> &'static str {
        "File operations: content search, directory tree, structured read."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("search", "Search files by name or content", "everyday fs search [--content PATTERN] [--path P] [NAME-GLOB]"),
            ActionDoc::new("tree", "Show directory tree", "everyday fs tree [--path P] [--max-depth N]"),
            ActionDoc::new("read-json", "Read & pretty-print a JSON/TOML file", "everyday fs read-json <path>"),
        ]
    }

    async fn execute(&self, action: &str, _args: &[String]) -> Result<Output> {
        match action {
            "search" | "tree" | "read-json" => Err(AgentError::NotImplemented(format!(
                "fs {action} — ignore/walkdir integration pending (see task_plan Phase 6)"
            ))),
            other => Err(AgentError::UnknownAction(format!("fs {other}"))),
        }
    }
}
