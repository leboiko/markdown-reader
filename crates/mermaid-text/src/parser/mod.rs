//! Mermaid diagram parsers.
//!
//! Supports `graph`/`flowchart` and `sequenceDiagram` syntax.

pub mod flowchart;
pub mod sequence;

pub use flowchart::parse;
