//! Rendering pipeline: graph + positions → Unicode string.

pub mod er;
pub mod pie;
pub mod sequence;
pub mod unicode;

pub use unicode::{render, render_color};
