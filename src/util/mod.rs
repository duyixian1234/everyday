//! Generic utility layer (`util`).
//!
//! Houses domain-agnostic, pure helper functions: CLI argument parsing
//! ([`args`]), short unique ID generation ([`id`]), and render-mode probing
//! ([`json_mode`]).
//!
//! Distinguished from `crate::shared` (stateful, IO-bearing shared
//! facilities): `util` only holds small, pure helpers.

pub mod args;
pub mod datetime;
pub mod id;
pub mod json_mode;
