pub mod renderer;

use std::cell::Cell;

use ratatui::text::{Span, Text};

/// Position and metadata of a hyperlink within a rendered text block.
///
/// `line` is 0-indexed relative to the start of the containing `DocBlock::Text`.
/// `col_start` and `col_end` are byte-column offsets in the rendered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub line: u32,
    pub col_start: u16,
    pub col_end: u16,
    pub url: String,
    pub text: String,
}

/// A heading anchor within a rendered text block.
///
/// `anchor` is the GitHub-style slug derived from the heading text.
/// `line` is 0-indexed within the containing `DocBlock::Text` block; it is
/// converted to an absolute display line in `MarkdownViewState::load`.
#[derive(Debug, Clone)]
pub struct HeadingAnchor {
    pub anchor: String,
    pub line: u32,
}

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
    /// A run of styled ratatui lines, with any hyperlinks and heading anchors
    /// found within them.
    Text {
        text: Text<'static>,
        links: Vec<LinkInfo>,
        heading_anchors: Vec<HeadingAnchor>,
    },
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
            DocBlock::Text { text, .. } => text.lines.len() as u32,
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

/// Convert a heading's visible text to a GitHub-style anchor slug.
///
/// Algorithm:
/// 1. Lowercase the text.
/// 2. Remove any character that is not alphanumeric, a space, or a hyphen.
/// 3. Replace spaces with hyphens.
/// 4. Collapse consecutive hyphens into one.
///
/// Non-ASCII letters pass step 2 unchanged (GitHub preserves them). Characters
/// such as `'` and `.` are stripped. Duplicate anchors are not deduplicated
/// here; callers that need disambiguation must track a counter themselves.
///
/// # Examples
///
/// ```
/// # use markdown_tui_explorer::markdown::heading_to_anchor;
/// assert_eq!(heading_to_anchor("Installation Guide"), "installation-guide");
/// assert_eq!(heading_to_anchor("What's New?"), "whats-new");
/// assert_eq!(heading_to_anchor("API v2.0"), "api-v20");
/// ```
pub fn heading_to_anchor(text: &str) -> String {
    let lower = text.to_lowercase();
    // Keep alphanumeric (any script), hyphens, and spaces; drop everything else.
    let filtered: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == ' ')
        .collect();
    // Spaces → hyphens, then collapse runs of consecutive hyphens.
    let hyphenated = filtered.replace(' ', "-");
    let mut slug = String::with_capacity(hyphenated.len());
    let mut prev_hyphen = false;
    for ch in hyphenated.chars() {
        if ch == '-' {
            if !prev_hyphen {
                slug.push(ch);
            }
            prev_hyphen = true;
        } else {
            slug.push(ch);
            prev_hyphen = false;
        }
    }
    // Strip leading/trailing hyphens that may appear after filtering.
    slug.trim_matches('-').to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::renderer::render_markdown;
    use crate::theme::Palette;

    fn palette() -> Palette {
        Palette::from_theme(crate::theme::Theme::Default)
    }

    // ── heading_to_anchor ────────────────────────────────────────────────────

    #[test]
    fn anchor_plain_words() {
        assert_eq!(heading_to_anchor("Installation Guide"), "installation-guide");
    }

    #[test]
    fn anchor_apostrophe_stripped() {
        assert_eq!(heading_to_anchor("What's New?"), "whats-new");
    }

    #[test]
    fn anchor_dot_stripped() {
        assert_eq!(heading_to_anchor("API v2.0"), "api-v20");
    }

    #[test]
    fn anchor_already_lowercase() {
        assert_eq!(heading_to_anchor("hello world"), "hello-world");
    }

    #[test]
    fn anchor_consecutive_spaces_collapse() {
        assert_eq!(heading_to_anchor("A  B"), "a-b");
    }

    #[test]
    fn anchor_empty() {
        assert_eq!(heading_to_anchor(""), "");
    }

    // ── Link info collection ─────────────────────────────────────────────────

    #[test]
    fn link_info_internal_anchor() {
        let md = "[Installation](#installation)\n";
        let blocks = render_markdown(md, &palette());
        let link = match &blocks[0] {
            DocBlock::Text { links, .. } => links.first().expect("link expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(link.url, "#installation");
        assert_eq!(link.text, "Installation");
        assert_eq!(link.line, 0);
        // col_start = 0 (nothing before), col_end = len("Installation") = 12
        assert_eq!(link.col_start, 0);
        assert_eq!(link.col_end, 12);
    }

    #[test]
    fn link_info_external_url_preserved() {
        let md = "[Rust](https://rust-lang.org)\n";
        let blocks = render_markdown(md, &palette());
        let link = match &blocks[0] {
            DocBlock::Text { links, .. } => links.first().expect("link expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(link.url, "https://rust-lang.org");
    }

    #[test]
    fn heading_anchor_collected() {
        let md = "# Installation Guide\n\nsome text\n";
        let blocks = render_markdown(md, &palette());
        let anchor = match &blocks[0] {
            DocBlock::Text { heading_anchors, .. } => {
                heading_anchors.first().expect("anchor expected")
            }
            _ => panic!("expected Text block"),
        };
        assert_eq!(anchor.anchor, "installation-guide");
        assert_eq!(anchor.line, 0);
    }
}
