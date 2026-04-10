pub mod renderer;

use ratatui::text::Text;

/// Opaque identifier for a mermaid diagram block, derived from a hash of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MermaidBlockId(pub u64);

/// Opaque stable identifier for a table block, derived from a hash of its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableBlockId(pub u64);

/// Structured representation of a markdown table, parsed once at render time.
///
/// The `natural_widths` field stores the maximum display-character width per column
/// across all rows (header included), measured with `unicode_width`. This is used
/// by both the in-document fair-share renderer and the modal full-width renderer.
#[derive(Debug, Clone)]
pub struct TableBlock {
    pub id: TableBlockId,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
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
#[derive(Debug)]
pub enum DocBlock {
    /// A run of styled ratatui lines.
    Text(Text<'static>),
    /// A reserved space for a mermaid diagram image.
    Mermaid {
        id: MermaidBlockId,
        /// The raw mermaid source, kept so the fallback renderer can display it.
        source: String,
    },
    /// A parsed markdown table rendered inline with fair-share column widths.
    Table(TableBlock),
}

impl DocBlock {
    /// Number of display lines this block occupies.
    pub fn height(&self) -> u32 {
        match self {
            DocBlock::Text(t) => t.lines.len() as u32,
            DocBlock::Mermaid { .. } => MERMAID_BLOCK_HEIGHT,
            DocBlock::Table(t) => t.rendered_height,
        }
    }
}

/// Fixed display-line height reserved for each mermaid diagram.
pub const MERMAID_BLOCK_HEIGHT: u32 = 20;
