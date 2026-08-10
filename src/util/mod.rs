//! Generic utility layer (`util`).
//!
//! Houses domain-agnostic, pure helper functions: CLI argument parsing
//! ([`args`]), short unique ID generation ([`id`]), render-mode probing
//! ([`json_mode`]), and leveled logging setup ([`logging`]).
//!
//! Distinguished from `crate::shared` (stateful, IO-bearing shared
//! facilities): `util` only holds small, pure helpers.

pub mod args;
pub mod datetime;
pub mod id;
pub mod json_mode;
pub mod logging;
pub mod strings;
