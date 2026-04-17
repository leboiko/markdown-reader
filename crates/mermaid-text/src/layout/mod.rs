//! Layout algorithms and the character grid canvas.

pub mod grid;
pub mod layered;

pub use grid::Grid;
pub use layered::{LayoutConfig, layout};
