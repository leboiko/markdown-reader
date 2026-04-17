pub mod highlight;
pub mod math;
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
    /// 0-indexed source line where the opening `|` row of the table appears.
    pub source_line: u32,
    /// Source lines for each logical row: index 0 is the header, indices
    /// `1..=rows.len()` are body rows.  Length equals `1 + rows.len()`.
    ///
    /// Used by `source_line_at` to map a cursor position inside a rendered
    /// table back to the exact markdown source row, so `enter_edit_mode` drops
    /// the editor cursor on the right line.
    pub row_source_lines: Vec<u32>,
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
        /// Parallel to `text.lines`: 0-indexed source line for each rendered row.
        /// `source_lines.len() == text.lines.len()` is a maintained invariant.
        source_lines: Vec<u32>,
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
        /// 0-indexed source line where the opening ` ```mermaid ` fence appears.
        source_line: u32,
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
            ..
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

/// Map a cursor position inside a rendered table to the source line of the
/// corresponding markdown row.
///
/// The inline table renderer produces a fixed layout:
///
/// ```text
/// Row | Content
/// ----+---------
///   0 | ┌──┬──┐      top border
///   1 | │ H │...│    header
///   2 | ├──┼──┤      separator
///   3 | │ a │...│    body[0]
///   4 | │ b │...│    body[1]
///  ...
///   N | └──┴──┘      bottom border
/// N+1 | [expand…]    optional truncation hint
/// ```
///
/// `local` is the 0-based row within the block. Border rows fall back to the
/// nearest content row (header or last body). If `row_source_lines` is shorter
/// than expected (e.g. in tests with stub data), `source_line` is used as a
/// safe fallback.
fn table_row_source_line(t: &TableBlock, local: usize) -> u32 {
    // Row indices of rendered elements.
    let header_idx: usize = 1; // after top border
    let first_body_idx: usize = 3; // after header + separator
    let last_body_idx: usize = first_body_idx + t.rows.len();

    match local {
        // Top border — fall back to header source line.
        i if i < header_idx => t.row_source_lines.first().copied().unwrap_or(t.source_line),
        // Header row.
        i if i == header_idx => t.row_source_lines.first().copied().unwrap_or(t.source_line),
        // Separator — header fallback.
        i if i < first_body_idx => t.row_source_lines.first().copied().unwrap_or(t.source_line),
        // Body row.
        i if i < last_body_idx => {
            let body_index = i - first_body_idx;
            t.row_source_lines
                .get(1 + body_index)
                .copied()
                .unwrap_or(t.source_line)
        }
        // Bottom border / truncation hint — last body row fallback.
        _ => t.row_source_lines.last().copied().unwrap_or(t.source_line),
    }
}

/// Walk `blocks` and return the 0-indexed source line that corresponds to
/// `logical_line` (the viewer's absolute rendered-line coordinate).
///
/// This is the bridge between the cursor position and the edtui editor row.
///
/// # Returns
///
/// The 0-indexed markdown source line.  Returns `0` when `logical_line` falls
/// beyond all blocks (documents shorter than expected due to race conditions).
pub fn source_line_at(blocks: &[DocBlock], logical_line: u32) -> u32 {
    let mut offset = 0u32;
    for block in blocks {
        let h = block.height();
        if logical_line < offset + h {
            let local = (logical_line - offset) as usize;
            return match block {
                DocBlock::Text { source_lines, .. } => {
                    source_lines.get(local).copied().unwrap_or(0)
                }
                DocBlock::Mermaid {
                    source_line,
                    source,
                    ..
                } => {
                    if local == 0 {
                        // local == 0 is the fence line itself.
                        *source_line
                    } else {
                        // Content lines: fence + 1 + K, clamped to the last content
                        // line so the closing fence and anything beyond still map to
                        // the last real source line inside the block.
                        //
                        // `source.lines().count()` is O(n) in the source length, but
                        // this function is only called from `enter_edit_mode`, never
                        // per frame, so the cost is acceptable.
                        let content_count = source.lines().count() as u32;
                        let content_offset =
                            (local as u32 - 1).min(content_count.saturating_sub(1));
                        *source_line + 1 + content_offset
                    }
                }
                DocBlock::Table(t) => table_row_source_line(t, local),
            };
        }
        offset += h;
    }
    0
}

/// Inverse of [`source_line_at`]: locate the first rendered logical line that
/// originates from source line `target_source` (0-indexed).
///
/// Returns `None` only when `blocks` is empty (no candidate exists at all).
/// For non-empty block lists, out-of-range or gap targets return the closest
/// preceding rendered line whose recorded source number is `<= target_source`.
///
/// # Arguments
///
/// * `blocks` – the rendered document block list.
/// * `target_source` – 0-indexed source line to locate.
///
/// # Examples
///
/// ```
/// # use markdown_tui_explorer::markdown::{DocBlock, logical_line_at_source};
/// # use ratatui::text::{Line, Span, Text};
/// let block = DocBlock::Text {
///     text: Text::from(vec![
///         Line::from(Span::raw("a")),
///         Line::from(Span::raw("b")),
///     ]),
///     links: vec![],
///     heading_anchors: vec![],
///     source_lines: vec![0, 1],
/// };
/// assert_eq!(logical_line_at_source(&[block], 1), Some(1));
/// ```
pub fn logical_line_at_source(blocks: &[DocBlock], target_source: u32) -> Option<u32> {
    // A rendered logical line can span multiple source lines (pulldown-cmark
    // joins soft breaks, inline formatting, etc.), so exact matching on
    // `source_lines` would miss any target that lands inside a joined line.
    //
    // Instead, track the last rendered line whose recorded source number is
    // `<= target_source` — that is the line that visually contains the target.
    // Exact hits inside a Mermaid or Table block short-circuit immediately
    // since those blocks have unambiguous per-row source lines.
    let mut offset = 0u32;
    let mut best: Option<u32> = None;

    for block in blocks {
        let height = block.height();
        match block {
            DocBlock::Text { source_lines, .. } => {
                // Do NOT break early here.  List item End-events can cause
                // non-monotonic source_lines (e.g. [..., 165, 160, 167, ...])
                // so a break would skip valid candidates after a dip.
                //
                // However, for EXACT matches we return the FIRST occurrence
                // immediately.  The same source line can appear at multiple
                // positions (heading + its trailing blank, or a list-End
                // dip back to the list's start line).  The first occurrence
                // is always the actual content; later occurrences are
                // rendering artifacts.
                for (i, &s) in source_lines.iter().enumerate() {
                    if s == target_source {
                        return Some(offset + i as u32);
                    }
                    if s <= target_source {
                        best = Some(offset + i as u32);
                    }
                }
            }
            DocBlock::Mermaid {
                source_line,
                source,
                ..
            } => {
                let content_count = source.lines().count() as u32;
                let block_end_source = *source_line + 1 + content_count;
                if target_source >= *source_line && target_source < block_end_source {
                    let local = target_source - *source_line;
                    return Some(offset + local.min(height.saturating_sub(1)));
                }
                if *source_line <= target_source {
                    // The block starts before the target but doesn't contain
                    // it — record its first row as a fallback candidate.
                    best = Some(offset);
                }
            }
            DocBlock::Table(t) => {
                // Table rows are emitted in document order (monotonically
                // increasing source lines), so breaking early here is safe:
                // no later row can have a smaller source number than the
                // current one.
                for (row_idx, &s) in t.row_source_lines.iter().enumerate() {
                    let rendered_row = if row_idx == 0 {
                        1u32 // header is at rendered index 1
                    } else {
                        3 + (row_idx - 1) as u32 // body rows start at rendered index 3
                    };
                    // row_idx increases monotonically and so does rendered_row;
                    // no later row can fit either, so stop scanning.
                    if rendered_row >= height {
                        break;
                    }
                    if s == target_source {
                        return Some(offset + rendered_row);
                    }
                    if s <= target_source {
                        best = Some(offset + rendered_row);
                    } else {
                        break;
                    }
                }
            }
        }
        offset += height;
    }
    best
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

    // ── logical_line_at_source ───────────────────────────────────────────────

    /// Helper to build a `DocBlock::Text` with explicit source-line mapping.
    fn text_block_with_sources(content: &[&str], sources: &[u32]) -> DocBlock {
        let lines: Vec<ratatui::text::Line<'static>> = content
            .iter()
            .map(|s| ratatui::text::Line::from(ratatui::text::Span::raw(s.to_string())))
            .collect();
        DocBlock::Text {
            text: ratatui::text::Text::from(lines),
            links: vec![],
            heading_anchors: vec![],
            source_lines: sources.to_vec(),
        }
    }

    #[test]
    fn logical_line_at_source_finds_text_line() {
        // Single Text block: source lines [0, 1, 2] map to logical lines 0, 1, 2.
        let block = text_block_with_sources(&["a", "b", "c"], &[0, 1, 2]);
        assert_eq!(logical_line_at_source(&[block], 1), Some(1));
    }

    #[test]
    fn logical_line_at_source_across_blocks() {
        // Two Text blocks: first covers source [0, 1], second covers source [3, 4, 5].
        let b1 = text_block_with_sources(&["a", "b"], &[0, 1]);
        let b2 = text_block_with_sources(&["d", "e", "f"], &[3, 4, 5]);
        // Source line 4 is in the second block at local index 1.
        // First block has height 2, so absolute offset is 2 + 1 = 3.
        assert_eq!(logical_line_at_source(&[b1, b2], 4), Some(3));
    }

    #[test]
    fn logical_line_at_source_table_header() {
        use crate::markdown::{TableBlock, TableBlockId};
        // Table with row_source_lines = [5, 7, 8]:
        //   rendered row 0: top border
        //   rendered row 1: header (source 5)
        //   rendered row 2: separator
        //   rendered row 3: body[0] (source 7)
        //   rendered row 4: body[1] (source 8)
        //   rendered row 5: bottom border
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![vec![ratatui::text::Span::raw("H")]],
            rows: vec![
                vec![vec![ratatui::text::Span::raw("a")]],
                vec![vec![ratatui::text::Span::raw("b")]],
            ],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 6,
            source_line: 5,
            row_source_lines: vec![5, 7, 8],
        });
        // Source line 5 (header) should map to rendered row 1.
        assert_eq!(logical_line_at_source(&[block], 5), Some(1));
    }

    #[test]
    fn logical_line_at_source_table_body() {
        use crate::markdown::{TableBlock, TableBlockId};
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(1),
            headers: vec![vec![ratatui::text::Span::raw("H")]],
            rows: vec![
                vec![vec![ratatui::text::Span::raw("a")]],
                vec![vec![ratatui::text::Span::raw("b")]],
            ],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 6,
            source_line: 5,
            row_source_lines: vec![5, 7, 8],
        });
        // Source line 7 (first body row) should map to rendered row 3.
        assert_eq!(logical_line_at_source(&[block], 7), Some(3));
    }

    #[test]
    fn logical_line_at_source_mermaid_inside() {
        use std::cell::Cell;
        // Mermaid fence at source line 10, content "a\nb\nc" (3 lines).
        // Source range: [10, 14) — fence (10), a (11), b (12), c (13).
        // Block height set to 4 to cover the fence + content.
        let block = DocBlock::Mermaid {
            id: crate::markdown::MermaidBlockId(0),
            source: "a\nb\nc".to_string(),
            cell_height: Cell::new(4),
            source_line: 10,
        };
        // Source line 12 = fence + 2 → local index 2 → logical line 0 + 2 = 2.
        assert_eq!(logical_line_at_source(&[block], 12), Some(2));
    }

    #[test]
    fn logical_line_at_source_overshoot_falls_back_to_last_line() {
        // A single Text block covering source lines 0–2.
        // Asking for source line 99 beyond the block's last recorded source
        // line should fall back to the closest earlier candidate — the last
        // rendered line whose source <= target.
        let block = text_block_with_sources(&["x", "y", "z"], &[0, 1, 2]);
        assert_eq!(logical_line_at_source(&[block], 99), Some(2));
    }

    #[test]
    fn logical_line_at_source_non_monotonic_text_block() {
        // List items + End-of-list dip: source_lines dips from 165 back to 160.
        // This mirrors the real renderer output that caused the OAB jump bug.
        let block = text_block_with_sources(&["a", "b", "c"], &[165, 160, 167]);
        // target=163 should land on index 1 (s=160, the largest s <= 163).
        assert_eq!(logical_line_at_source(&[block], 163), Some(1));
    }

    /// When the same source line appears at multiple positions (e.g. a
    /// heading line + its trailing blank, or a list-End dip), the FIRST
    /// occurrence must win — it's the actual content, not the artifact.
    #[test]
    fn logical_line_at_source_duplicate_source_line_returns_first() {
        // Source line 306 appears at indices 0 and 3 (simulating a heading
        // at index 0 and a list-End dip at index 3).
        let block = text_block_with_sources(
            &["heading", "para1", "para2", "blank-after-list"],
            &[306, 307, 308, 306],
        );
        // Must return the FIRST index (0), not the last (3).
        assert_eq!(logical_line_at_source(&[block], 306), Some(0));
    }

    /// Same scenario across two blocks: block A has the real line, block B
    /// has the duplicate from a dip. The function must return block A's
    /// match.
    #[test]
    fn logical_line_at_source_duplicate_across_blocks_returns_first() {
        let b1 = text_block_with_sources(&["real content"], &[306]);
        let b2 = text_block_with_sources(
            &["other", "dip-artifact"],
            &[310, 306],
        );
        // b1 height=1, b2 starts at offset 1. Target 306 is in b1 at 0.
        assert_eq!(logical_line_at_source(&[b1, b2], 306), Some(0));
    }

    #[test]
    fn logical_line_at_source_target_beyond_any_block_is_none() {
        // With no blocks at all there is no candidate anywhere, so None.
        assert_eq!(logical_line_at_source(&[], 5), None);
    }

    #[test]
    fn logical_line_at_source_target_inside_joined_paragraph() {
        // A paragraph whose source spans lines 5–7 but renders as a single
        // joined line (pulldown-cmark merges soft breaks). Only line 5 is
        // recorded; asking for 5, 6, or 7 must still land on the same
        // rendered line. DocBlock is not Clone, so rebuild per assertion.
        for target in [5u32, 6, 7] {
            let block = text_block_with_sources(&["joined paragraph"], &[5]);
            assert_eq!(
                logical_line_at_source(&[block], target),
                Some(0),
                "target source line {target} should land on the joined paragraph's rendered line 0",
            );
        }
    }

    #[test]
    fn logical_line_at_source_between_blocks_lands_on_previous_last_line() {
        // First block source [0, 1], second block source [10, 11]. Source
        // line 5 falls in the gap; it should land on the last line of the
        // first block (closest candidate at or before 5).
        let b1 = text_block_with_sources(&["a", "b"], &[0, 1]);
        let b2 = text_block_with_sources(&["c", "d"], &[10, 11]);
        assert_eq!(logical_line_at_source(&[b1, b2], 5), Some(1));
    }
}
