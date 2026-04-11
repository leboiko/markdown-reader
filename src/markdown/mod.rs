pub mod renderer;

use std::cell::Cell;

use ratatui::text::{Span, Text};

/// Opaque identifier for a mermaid diagram block, derived from a hash of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MermaidBlockId(pub u64);

/// Opaque stable identifier for a table block, derived from a hash of its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableBlockId(pub u64);

/// One cell's content as a sequence of styled spans.
pub type CellSpans = Vec<Span<'static>>;

/// Structured representation of a markdown table, parsed once at render time.
///
/// Each cell is a `Vec<Span<'static>>` preserving inline styling (bold, italic,
/// code, links, strikethrough). `natural_widths` is measured from the sum of
/// `unicode_width` across each cell's spans and is used by both renderers.
#[derive(Debug, Clone)]
pub struct TableBlock {
    pub id: TableBlockId,
    pub headers: Vec<CellSpans>,
    pub rows: Vec<Vec<CellSpans>>,
    pub alignments: Vec<pulldown_cmark::Alignment>,
    /// Maximum display width of any cell per column, including the header.
    pub natural_widths: Vec<usize>,
    /// Cached display-line height; updated lazily when the layout width changes.
    pub rendered_height: u32,
}

/// A single rendered block in a document.
///
/// Documents are modelled as a sequence of these blocks rather than a flat
/// `Text` so that mermaid diagrams and wide tables can be handled independently
/// of the text paragraph.
///
/// `DocBlock` is intentionally not `Send` because the `Mermaid` variant contains
/// a `Cell<u32>`. All `DocBlock` values live on the main async task and are never
/// moved to a worker thread, so this is safe.
#[derive(Debug)]
pub enum DocBlock {
    /// A run of styled ratatui lines.
    Text(Text<'static>),
    /// A reserved space for a mermaid diagram image.
    Mermaid {
        id: MermaidBlockId,
        /// The raw mermaid source, kept so the fallback renderer can display it.
        source: String,
        /// Current reserved height in display lines. Written by
        /// `update_mermaid_heights` each frame from the cache, read by `height()`.
        /// `Cell` avoids `&mut` while still allowing interior mutation during
        /// a shared-reference iteration over the block list.
        cell_height: Cell<u32>,
    },
    /// A parsed markdown table rendered inline with fair-share column widths.
    Table(TableBlock),
}

impl DocBlock {
    /// Number of display lines this block occupies.
    pub fn height(&self) -> u32 {
        match self {
            DocBlock::Text(t) => t.lines.len() as u32,
            DocBlock::Mermaid { cell_height, .. } => cell_height.get(),
            DocBlock::Table(t) => t.rendered_height,
        }
    }
}

/// Synchronise the `cell_height` of every `Mermaid` block in `blocks` with the
/// current cache. Call this before summing `total_lines` so scroll math reflects
/// whatever the cache knows at the time of the draw.
pub fn update_mermaid_heights(blocks: &[DocBlock], cache: &crate::mermaid::MermaidCache) {
    for block in blocks {
        if let DocBlock::Mermaid { id, source, cell_height } = block {
            cell_height.set(cache.height(id, source));
        }
    }
}

/// Return the total display-column width of a cell's spans.
pub fn cell_display_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// Flatten a cell's spans to a plain string (for search and modal wrapping).
pub fn cell_to_string(spans: &[Span<'static>]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

