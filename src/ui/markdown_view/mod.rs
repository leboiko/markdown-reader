//! Markdown preview panel.
//!
//! The module is split into focused submodules; everything that external
//! callers need is re-exported from here so the public API is unchanged.

mod draw;
mod gutter;
mod highlight;
mod mermaid_draw;
mod state;
mod tests;

// Public API — re-export everything callers access via `crate::ui::markdown_view::*`.
pub use draw::draw;
pub use highlight::extract_line_text_range;
pub use state::{MarkdownViewState, TableLayout, VisualMode, VisualRange, WrappedTextLayout};
