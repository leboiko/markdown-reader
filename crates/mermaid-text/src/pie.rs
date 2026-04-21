//! Data model for Mermaid `pie` charts.
//!
//! `pie` is the simplest of Mermaid's diagram types: an optional title, an
//! optional `showData` flag, and a list of labelled positive numeric slices.
//! In the web Mermaid renderer these become a circular pie chart with
//! coloured slices; in this crate they render as a horizontal bar chart
//! (much more legible in monospace text — see [`crate::render::pie`]).

/// One slice of a [`PieChart`]. The label is the user-supplied display
/// string (everything between the first pair of double-quotes on a data
/// line) and `value` is its numeric weight (`f64` to support decimal
/// inputs like `"X" : 12.5`).
#[derive(Debug, Clone, PartialEq)]
pub struct PieSlice {
    pub label: String,
    pub value: f64,
}

/// A parsed `pie` chart.
///
/// Constructed by [`crate::parser::pie::parse`] and consumed by
/// [`crate::render::pie::render`]. `title` is the text after `pie title …`
/// (or `None` when omitted). `show_data` mirrors Mermaid's `showData`
/// keyword — when `true` the renderer includes the raw value next to
/// the percentage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PieChart {
    pub title: Option<String>,
    pub show_data: bool,
    pub slices: Vec<PieSlice>,
}

impl PieChart {
    /// Sum of all slice values. Used by the renderer to compute each
    /// slice's share of the whole. Always positive (the parser rejects
    /// non-positive totals at validation time).
    pub fn total(&self) -> f64 {
        self.slices.iter().map(|s| s.value).sum()
    }
}
