//! `mc` library crate.
//!
//! This re-exports the internal modules so integration tests under `tests/`
//! and downstream consumers can drive mc programmatically. The CLI entry
//! point lives in `src/main.rs` and uses these same modules.

pub mod api;
pub mod cli;
pub mod commands;
pub mod config;
pub mod data;
pub mod entity;
pub mod error;
pub mod frontmatter;
pub mod html;
pub mod mcp;
pub mod template;
pub mod util;
