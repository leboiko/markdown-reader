//! Mermaid diagram parsers.
//!
//! Currently only `graph`/`flowchart` syntax is supported.

pub mod flowchart;

pub use flowchart::parse;
