//! Mermaid diagram parsers.
//!
//! Currently only flowchart syntax is supported (Phase 1).

pub mod flowchart;

pub use flowchart::parse;
