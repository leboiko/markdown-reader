//! Mermaid diagram parsers.
//!
//! Supports `graph`/`flowchart`, `sequenceDiagram`, `stateDiagram` /
//! `stateDiagram-v2`, `erDiagram`, `classDiagram`, and `journey` syntax.

pub mod class;
pub(crate) mod common;
pub mod er;
pub mod flowchart;
pub mod journey;
pub mod pie;
pub mod sequence;
pub mod state;

pub use flowchart::parse;
