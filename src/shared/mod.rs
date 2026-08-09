//! Shared infrastructure layer (`shared`).
//!
//! Hosts the low-level facilities reused across modules: config loading
//! [`config`], unified errors [`error`], and output rendering [`output`].
//!
//! Domain modules (see `crate::modules`) reach these via
//! `crate::{config, error, output, ...}` — those paths are re-exported at
//! the crate root by `main.rs`, so this layer's physical location is
//! transparent to upper layers.

pub mod config;
pub mod error;
pub mod middleware;
pub mod output;
pub mod request_context;
