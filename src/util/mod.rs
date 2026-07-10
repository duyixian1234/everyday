//! 通用工具层（util）。
//!
//! 收纳与具体领域无关的纯工具函数：命令行参数解析 [`args`]、
//! 短唯一 ID 生成 [`id`]、渲染模式探测 [`json_mode`]。
//!
//! 与 `crate::shared`（有状态/带 IO 的共享设施）区分：util 只放小而纯的 helper。

pub mod args;
pub mod id;
pub mod json_mode;
