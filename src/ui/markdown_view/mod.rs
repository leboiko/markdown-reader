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
mod visual_rows;

// Public API — re-export everything callers access via `crate::ui::markdown_view::*`.
// The `unused_imports` warning below is a false positive from the pre-existing
// E0761 ambiguity on the `app` module; all items here are used by external callers.
#[allow(unused_imports)]
pub use draw::draw;
#[allow(unused_imports)]
pub use highlight::extract_line_text_range;
#[allow(unused_imports)]
pub use state::{
    AbsoluteAnchor, AbsoluteLink, MarkdownViewState, TableLayout, VisualMode, VisualRange,
};
#[allow(unused_imports)]
pub use visual_rows::{line_visual_rows, visual_row_to_logical_line};
