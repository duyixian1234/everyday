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
/// 主程序通过 [`ModuleRegistry`] 按 name 查找 trait object 并调用 [`Executor::execute`]。
#[async_trait]
pub trait Executor: Send + Sync {
    /// 模块名（对应 CLI 的 `<module>`，如 `mail`、`cal`）。
    fn name(&self) -> &'static str;

    /// 一句话描述。
    fn description(&self) -> &'static str;

    /// 该模块支持的动作文档，用于 `everyday <module> --help`。
    fn actions(&self) -> Vec<ActionDoc>;

    /// 执行指定 action。
    ///
    /// - `action`：动作名（如 `list`、`send`、`status`）
    /// - `args`：剩余命令行参数（模块自行解析）
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
}

/// 动作文档。
#[derive(Debug, Clone)]
pub struct ActionDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
}

impl ActionDoc {
    pub const fn new(name: &'static str, description: &'static str, usage: &'static str) -> Self {
        Self {
            name,
            description,
            usage,
        }
    }
}

/// 模块注册表。
///
/// 构建时注入配置与（可选的）`--account` 覆盖，
/// 各模块按需读取自己所需的账户配置。
pub struct ModuleRegistry {
    modules: HashMap<&'static str, Box<dyn Executor>>,
}

impl ModuleRegistry {
    /// 根据配置构建所有模块。
    /// `account_override`：来自全局 `--account` flag，模块可选择性使用。
    pub fn build(config: Arc<Config>, account_override: Option<&str>) -> Result<Self> {
        let mut modules: HashMap<&'static str, Box<dyn Executor>> = HashMap::new();

        // 注册各模块。模块内部决定是否需要账户配置、是否容忍缺失。
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

        let _ = account_override; // 各模块按需通过 config 自行解析；此处保留参数以便未来扩展
        Ok(Self { modules })
    }

    /// 按名查找模块。
    pub fn get(&self, name: &str) -> Result<&dyn Executor> {
        self.modules
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| AgentError::ModuleNotFound(name.to_string()))
    }

    /// 列出所有已注册模块名。
    pub fn module_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.modules.keys().copied().collect();
        names.sort();
        names
    }
}

// ---- 模块子模块声明 ----
pub mod calendar;
pub mod email;
pub mod note;
pub mod rss;
pub mod todo;

/// 解析 `--flag value` 形式的简单参数。
/// 返回 (flags map, positional args)。
/// 模块可复用此工具函数，避免每个模块都引入 clap。
pub fn parse_simple_args(args: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            // --key=value
            if let Some((k, v)) = stripped.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                // --key value
                flags.insert(stripped.to_string(), args[i + 1].clone());
                i += 1;
            } else {
                // --flag (boolean)
                flags.insert(stripped.to_string(), "true".to_string());
            }
        } else {
            positional.push(a.clone());
        }
        i += 1;
    }
    (flags, positional)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_flags_and_positional() {
        // 约定：`--flag` 后跟非 `--` token 会被当作该 flag 的值；
        // 布尔 flag 必须后接另一个 `--flag` 或位于末尾。
        let args: Vec<String> = ["--unread", "--limit", "10", "list", "extra"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (flags, positional) = parse_simple_args(&args);
        assert_eq!(flags.get("unread"), Some(&"true".to_string()));
        assert_eq!(flags.get("limit"), Some(&"10".to_string()));
        assert_eq!(positional, vec!["list", "extra"]);
    }

    #[test]
    fn parse_key_eq_value() {
        let args: Vec<String> = ["--limit=5", "pos"].iter().map(|s| s.to_string()).collect();
        let (flags, positional) = parse_simple_args(&args);
        assert_eq!(flags.get("limit"), Some(&"5".to_string()));
        assert_eq!(positional, vec!["pos"]);
    }

    struct DummyModule;
    #[async_trait]
    impl Executor for DummyModule {
        fn name(&self) -> &'static str {
            "dummy"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn actions(&self) -> Vec<ActionDoc> {
            vec![]
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
