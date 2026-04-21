//! Mermaid diagram parsers.
//!
//! Supports `graph`/`flowchart`, `sequenceDiagram`, and `stateDiagram` /
//! `stateDiagram-v2` syntax.

pub(crate) mod common;
pub mod flowchart;
pub mod pie;
pub mod sequence;
pub mod state;

pub use flowchart::parse;
