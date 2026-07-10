//! CLI 参数定义（clap derive）。
//!
//! 采用扁平结构 + `trailing_var_arg`，精确匹配 `agents.md` 的
//! `everyday <module> <action> [options]` 形态：
//! - `module` / `action` 为位置参数
//! - `args` 捕获剩余参数，交给模块自行解析
//! - `--json` / `--account` 为全局 flag

use clap::Parser;

/// The Rust-powered hands for your AI Agent.
#[derive(Parser, Debug)]
#[command(
    name = "everyday",
    version,
    about = "The Rust-powered hands for your AI Agent",
    long_about = "Unified CLI: everyday <module> <action> [options].\nModules: mail, cal, rss, note, todo, bookmark, config."
)]
pub struct Cli {
    /// 输出纯净 JSON（AI Agent 交互主模式）。
    #[arg(long, global = true)]
    pub json: bool,

    /// 覆盖模块的默认账户。
    #[arg(long, global = true)]
    pub account: Option<String>,

    /// 模块名：mail | cal | rss | note | todo | config
    #[arg(required = true)]
    pub module: String,

    /// 动作名（如 list / send / status）。省略时显示该模块帮助。
    pub action: Option<String>,

    /// 传递给模块的剩余参数。
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}
