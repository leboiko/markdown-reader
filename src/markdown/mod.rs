pub mod renderer;

use ratatui::text::Text;

/// Opaque identifier for a mermaid diagram block, derived from a hash of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MermaidBlockId(pub u64);

/// A single rendered block in a document.
///
/// Documents are modelled as a sequence of these blocks rather than a flat
/// `Text` so that mermaid diagrams can be drawn as image widgets at their
/// correct Y offset, independently of the text paragraph.
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
}

impl DocBlock {
    /// Number of display lines this block occupies.
    pub fn height(&self) -> u32 {
        match self {
            DocBlock::Text(t) => t.lines.len() as u32,
            DocBlock::Mermaid { .. } => MERMAID_BLOCK_HEIGHT,
        }
    }
}

/// Fixed display-line height reserved for each mermaid diagram.
pub const MERMAID_BLOCK_HEIGHT: u32 = 20;
