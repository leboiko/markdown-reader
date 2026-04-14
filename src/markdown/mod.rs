pub mod highlight;
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
///
/// Returns `true` when at least one block's height changed, allowing callers to
/// skip expensive downstream work (like `recompute_positions`) when nothing moved.
pub fn update_mermaid_heights(blocks: &[DocBlock], cache: &crate::mermaid::MermaidCache) -> bool {
    let mut changed = false;
    for block in blocks {
        if let DocBlock::Mermaid {
            id,
            source,
            cell_height,
        } = block
        {
            let new_h = cache.height(id, source);
            if new_h != cell_height.get() {
                cell_height.set(new_h);
                changed = true;
            }
        }
    }
    changed
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
    use crate::theme::{Palette, Theme};

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn theme() -> Theme {
        Theme::Default
    }

    // ── heading_to_anchor ────────────────────────────────────────────────────

    #[test]
    fn anchor_plain_words() {
        assert_eq!(
            heading_to_anchor("Installation Guide"),
            "installation-guide"
        );
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
        let blocks = render_markdown(md, &palette(), theme());
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
        let blocks = render_markdown(md, &palette(), theme());
        let link = match &blocks[0] {
            DocBlock::Text { links, .. } => links.first().expect("link expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(link.url, "https://rust-lang.org");
    }

    #[test]
    fn heading_anchor_collected() {
        let md = "# Installation Guide\n\nsome text\n";
        let blocks = render_markdown(md, &palette(), theme());
        let anchor = match &blocks[0] {
            DocBlock::Text {
                heading_anchors, ..
            } => heading_anchors.first().expect("anchor expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(anchor.anchor, "installation-guide");
        assert_eq!(anchor.line, 0);
    }

    /// Compute absolute anchor positions using the same logic as
    /// `MarkdownViewState::recompute_positions`.
    fn absolute_anchor_positions(blocks: &[DocBlock]) -> Vec<(String, u32)> {
        let mut result = Vec::new();
        let mut offset = 0u32;
        for block in blocks {
            if let DocBlock::Text {
                heading_anchors, ..
            } = block
            {
                for ha in heading_anchors {
                    result.push((ha.anchor.clone(), offset + ha.line));
                }
            }
            offset += block.height();
        }
        result
    }

    /// Compute the ACTUAL display line of each heading by scanning every
    /// rendered line for the heading prefix characters used by the renderer.
    fn actual_heading_lines(blocks: &[DocBlock]) -> Vec<(String, u32)> {
        let mut result = Vec::new();
        let mut abs_line = 0u32;
        for block in blocks {
            if let DocBlock::Text { text, .. } = block {
                for line in &text.lines {
                    let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    // The renderer prefixes headings with "█ ", "▌ ", or "▎ ".
                    for prefix in &["█ ", "▌ ", "▎ "] {
                        if content.contains(prefix) {
                            let text_after_prefix = content
                                .split_once(prefix)
                                .map(|(_, t)| t)
                                .unwrap_or("")
                                .trim();
                            if !text_after_prefix.is_empty() {
                                let anchor = heading_to_anchor(text_after_prefix);
                                result.push((anchor, abs_line));
                            }
                            break;
                        }
                    }
                    abs_line += 1;
                }
            } else {
                abs_line += block.height();
            }
        }
        result
    }

    /// Verify that `visual_row_to_logical_line` correctly resolves a visual row
    /// that follows a wrapping paragraph to the expected logical line, rather
    /// than the naive `scroll_offset + visual_row` computation.
    ///
    /// Without the fix the old formula (`clicked_line = scroll_offset + row`)
    /// would return a line number shifted by the number of visual rows consumed
    /// by line-wrapping above the clicked position, causing the wrong link to
    /// be matched and navigation to go to the wrong heading.
    #[test]
    fn visual_row_wrapping_maps_to_correct_logical_line() {
        use crate::ui::markdown_view::visual_row_to_logical_line;

        // Build a document where the first paragraph is longer than
        // content_width so it wraps. The TOC links appear after the paragraph.
        // We use an explicit repeat so the test is independent of terminal width.
        let long_para: String = "word ".repeat(30); // 150 chars → wraps at any realistic width
        let md = format!(
            "# Title\n\n{long_para}\n\n- [Section A](#section-a)\n- [Section B](#section-b)\n\n## Section A\n\nText.\n\n## Section B\n\nMore.\n",
        );

        let blocks = render_markdown(&md, &palette(), theme());

        // With a content_width of 80 the long paragraph wraps to
        // ceil(150/80) = 2 visual rows.
        let content_width: u16 = 80;

        // Logical layout of the first (only) text block:
        //   line 0: "█ Title"
        //   line 1: ""  (blank after heading)
        //   line 2: long paragraph  ← wraps to 2 visual rows at width 80
        //   line 3: ""  (blank after paragraph)
        //   line 4: "• Section A"   link at logical line 4
        //   line 5: "• Section B"   link at logical line 5
        //   line 6: ""  (blank after list)
        //   line 7: "▌ Section A"
        //   ...

        // Visual rows (scroll_offset = 0):
        //   row 0: "█ Title"
        //   row 1: ""
        //   row 2: long paragraph (part 1)
        //   row 3: long paragraph (part 2)  ← WRAP ROW
        //   row 4: ""
        //   row 5: "• Section A"   ← logical line 4, visual row 5
        //   row 6: "• Section B"   ← logical line 5, visual row 6

        // Naive formula: clicked visual row 5 → clicked_line = 0 + 5 = 5
        //   → that would match "Section B" link, not "Section A"!
        // Correct formula: visual row 5 → logical line 4 → "Section A"

        let logical_line_for_section_a = visual_row_to_logical_line(&blocks, 0, 5, content_width);
        assert_eq!(
            logical_line_for_section_a, 4,
            "visual row 5 should map to logical line 4 (Section A), \
             not naive row 5 (Section B); naive formula is off by 1 wrap row"
        );

        let logical_line_for_section_b = visual_row_to_logical_line(&blocks, 0, 6, content_width);
        assert_eq!(
            logical_line_for_section_b, 5,
            "visual row 6 should map to logical line 5 (Section B)"
        );
    }

    /// Heading anchors must point to the display line that actually shows the
    /// heading text, so that scrolling to `anchor.line` brings the heading to
    /// the top of the viewport.
    ///
    /// This test renders a document with headings before and after a mermaid
    /// diagram, a table, and a fenced code block, then asserts that every
    /// anchor computed by `recompute_positions` matches the actual display line
    /// where the heading text appears.
    #[test]
    fn anchor_positions_match_actual_heading_lines_after_special_blocks() {
        let md = concat!(
            "# Title\n\n",
            "- [Section A](#section-a)\n",
            "- [Section B](#section-b)\n",
            "- [Section C](#section-c)\n",
            "- [Section D](#section-d)\n\n",
            "## Section A\n\n",
            "Some text.\n\n",
            "```mermaid\n",
            "graph LR\n",
            "    A-->B\n",
            "```\n\n",
            "## Section B\n\n",
            "More text.\n\n",
            "| Col1 | Col2 |\n",
            "|------|------|\n",
            "| a    | b    |\n\n",
            "## Section C\n\n",
            "Some code:\n\n",
            "```rust\n",
            "let x = 1;\n",
            "```\n\n",
            "## Section D\n\n",
            "Final text.\n",
        );

        let blocks = render_markdown(md, &palette(), theme());

        let recorded = absolute_anchor_positions(&blocks);
        let actual = actual_heading_lines(&blocks);

        // Every anchor we recorded must have a corresponding entry in `actual`
        // at the same display line.
        for (anchor, recorded_line) in &recorded {
            let found = actual
                .iter()
                .find(|(a, _)| a == anchor)
                .unwrap_or_else(|| panic!("no heading found for anchor '{anchor}'"));
            assert_eq!(
                *recorded_line, found.1,
                "anchor '{anchor}': recorded line {recorded_line} != actual heading line {}",
                found.1,
            );
        }
    }
}
