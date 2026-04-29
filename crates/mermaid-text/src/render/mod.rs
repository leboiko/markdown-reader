//! Rendering pipeline: graph + positions → Unicode string.

pub mod box_table;
pub mod class;
pub mod er;
pub mod gantt;
pub mod git_graph;
pub mod journey;
pub mod mindmap;
pub mod pie;
pub mod sequence;
pub mod timeline;
pub mod unicode;

pub use unicode::{render, render_color};
