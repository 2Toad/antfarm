//! The Claude Code adapter: file formats, parsing, and the live source.

pub mod backfill;
pub mod cost;
pub mod detail;
pub mod files;
pub mod line;
pub mod locate;
pub mod parse;
pub mod price;
pub mod reduce;
pub mod registry;
pub mod source;
pub mod subagents;
pub mod workflows;

pub use source::{ClaudeCodeSource, SourceConfig, SourceStats, Wants};
