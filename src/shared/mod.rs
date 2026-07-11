//! 共享基础设施层（shared）。
//!
//! 收纳跨模块复用的底层设施：配置加载 [`config`]、统一错误 [`error`]、
//! 输出渲染 [`output`]、Notion 底层客户端 [`notion_client`]。
//!
//! 领域模块（见 `crate::modules`）通过 `crate::{config, error, output, ...}`
//! 访问这些设施——这些路径由 `main.rs` 在 crate 根做 re-export，故本层的
//! 物理位置对上层调用透明。

pub mod config;
pub mod error;
pub mod keyring_user;
pub mod notion_client;
pub mod output;
