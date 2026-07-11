//! 模块层：定义 [`Executor`] trait 与 [`ModuleRegistry`]。
//!
//! 每个功能模块（邮件、日历、RSS）实现 `Executor`，
//! 主程序只通过 `Box<dyn Executor>` 调度，保持 `main.rs` 极简。
//!
//! 定位：everyday 是 AI Agent 连接外部世界（邮件/日历/资讯）的统一接口，
//! 不内置文件搜索、HTTP、系统监控等代理可用 shell 直接完成的通用能力。

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::output::Output;

/// 模块执行器 trait。
///
/// 模块自身持有配置（构造时注入对应账户配置）。
/// 主程序通过 [`ModuleRegistry`] 按 name 查找 trait object 并调用 [`Executor::execute`].
#[async_trait]
pub trait Executor: Send + Sync {
    /// 一句话描述。
    fn description(&self) -> &'static str;

    /// 返回该模块的参数结构声明（clap 子命令化的单一事实来源）。
    ///
    /// 由 `cli.rs` 据此构建 `clap::Command` 树（module → action → flags），
    /// 模块自身无需感知 clap；`--account` 是全局 flag，不在此声明。
    fn module_arg_spec(&self) -> ModuleArgSpec;

    /// 执行指定 action。
    ///
    /// - `action`：动作名（如 `list`、`send`、`status`）
    /// - `args`：剩余命令行参数（模块自行解析）
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
}

/// clap 子命令化：每个模块以「数据」形式声明自己的参数结构，
/// 由 `cli.rs` 统一转成 `clap::Command` 树。单一事实来源，避免散落在 execute 里重复解析。
#[derive(Debug, Clone, Copy)]
pub enum ArgKind {
    /// 取值 flag：`--name VALUE`
    Value,
    /// 布尔开关：`--name`（无值）
    Bool,
    /// 可重复取值 flag：`--name V` 可多次，收集为列表（如 note 的 `--prop`）
    Multi,
}

/// 单个参数声明。
pub struct ArgSpec {
    pub name: &'static str,
    pub help: &'static str,
    pub kind: ArgKind,
}

/// 位置参数形态。
#[derive(Debug, Clone, Copy)]
pub enum Positional {
    /// 无位置参数（纯 flag 命令）。
    None,
    /// 恰好 N 个位置参数（如 `config set <path> <value>` 为 `Exactly(2)`）。
    Exactly(u8),
    /// 可选单个位置参数（0 或 1，如 `note read [<page_id>]`）。
    OptionalSingle,
}

/// 单个 action（子命令）的参数声明。
pub struct ActionArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
    pub args: &'static [ArgSpec],
    /// 位置参数声明（如 `config set <path> <value>`、`note read <page_id>`）。
    /// 位置参数统一以 `args` 这个 clap id 捕获，由 `matches_to_args` 原样还原。
    pub positional: Positional,
}

/// 模块级参数声明（clap 子命令化的单一事实来源）。
pub struct ModuleArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub actions: &'static [ActionArgSpec],
}

/// 模块注册表。
///
/// 构建时注入配置与（可选的）`--account` 覆盖，
/// 各模块按需读取自己所需的账户配置。
pub struct ModuleRegistry {
    pub(crate) modules: HashMap<&'static str, Box<dyn Executor>>,
}

impl ModuleRegistry {
    /// 根据配置构建所有模块。
    pub fn build(config: Arc<Config>) -> Result<Self> {
        let mut modules: HashMap<&'static str, Box<dyn Executor>> = HashMap::new();

        // 注册各模块。模块内部决定是否需要账户配置、是否容忍缺失。
        modules.insert(
            "config",
            Box::new(crate::modules::config::ConfigModule::new()),
        );
        modules.insert(
            "mail",
            Box::new(crate::modules::email::EmailModule::new(config.clone())),
        );
        modules.insert(
            "cal",
            Box::new(crate::modules::calendar::CalendarModule::new(
                config.clone(),
            )),
        );
        modules.insert(
            "rss",
            Box::new(crate::modules::rss::RssModule::new(config.clone())),
        );
        modules.insert(
            "note",
            Box::new(crate::modules::note::NoteModule::new(config.clone())),
        );
        modules.insert(
            "todo",
            Box::new(crate::modules::todo::TodoModule::new(config.clone())),
        );
        modules.insert(
            "bookmark",
            Box::new(crate::modules::bookmark::BookmarkModule::new(
                config.clone(),
            )),
        );
        modules.insert(
            "timeline",
            Box::new(crate::modules::timeline::TimelineModule::new(
                config.clone(),
            )),
        );

        Ok(Self { modules })
    }

    /// 按名查找模块。
    pub fn get(&self, name: &str) -> Result<&dyn Executor> {
        self.modules
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| AgentError::ModuleNotFound(name.to_string()))
    }
}

// ---- 模块子模块声明 ----
pub mod bookmark;
pub mod bookmark_local;
pub mod calendar;
pub mod config;
pub mod email;
pub mod email_cache;
pub mod email_pool;
pub mod local;
pub mod note;
pub mod note_local;
pub mod rss;
pub mod timeline;
pub mod todo;
pub mod todo_local;

/// 通用简单参数解析器。为兼容既有调用方（`crate::modules::parse_simple_args`），
/// 从 [`crate::util::args`] re-export。
pub use crate::util::args::parse_simple_args;

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyModule;
    #[async_trait]
    impl Executor for DummyModule {
        fn description(&self) -> &'static str {
            "test"
        }
        fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
            crate::modules::ModuleArgSpec {
                name: "dummy",
                description: "test",
                actions: &[],
            }
        }
        async fn execute(&self, _a: &str, _args: &[String]) -> Result<Output> {
            Ok(Output::text("ok"))
        }
    }

    #[tokio::test]
    async fn trait_object_dispatch_works() {
        let m: Box<dyn Executor> = Box::new(DummyModule);
        let out = m.execute("anything", &[]).await.unwrap();
        assert_eq!(out.render(crate::output::RenderMode::Text), "ok");
    }
}
