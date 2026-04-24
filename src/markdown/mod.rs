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
        /// Sum of visual rows each `text.lines` entry occupies at the current
        /// layout width. Defaults to `text.lines.len()` (the no-wrap case);
        /// set to the true visual count by [`update_text_visual_heights`] on
        /// every layout-width change. Read by [`DocBlock::height`] so scroll
        /// math advances by visual rows, not logical lines — without this, a
        /// long paragraph that wraps to N rows visually would still count as
        /// 1 line in `total_lines`, and scrolling past it would shift the
        /// table that follows by N-1 rows.
        ///
        /// `Cell` so the value can be updated through a shared `&DocBlock`
        /// reference during the immutable iteration in the draw loop.
        visual_height: Cell<u32>,
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
    /// Number of display rows this block occupies in the current viewport.
    ///
    /// Always returns visual rows, not logical lines: a Text block whose
    /// long Lines wrap returns the wrapped row count via `visual_height`.
    /// `update_text_visual_heights` must run after any `layout_width`
    /// change for this to be accurate; before then, the cell holds the
    /// pessimistic logical-line count from `Text::from(...)`.
    pub fn height(&self) -> u32 {
        match self {
            DocBlock::Text { visual_height, .. } => visual_height.get(),
            DocBlock::Mermaid { cell_height, .. } => cell_height.get(),
            DocBlock::Table(t) => t.rendered_height,
        }
    }
}

/// Sum each `Text` block's `visual_height` from its lines at the given
/// content width. Returns `true` when at least one block's height changed
/// so callers can skip downstream work (`recompute_positions`,
/// `total_lines` re-sum) when nothing moved.
///
/// Mermaid and Table blocks already store visual heights elsewhere
/// (`cell_height` / `rendered_height`) and are skipped here.
pub fn update_text_visual_heights(blocks: &[DocBlock], content_width: u16) -> bool {
    use crate::ui::markdown_view::visual_rows::line_visual_rows;
    let mut changed = false;
    for block in blocks {
        if let DocBlock::Text {
            text,
            visual_height,
            ..
        } = block
        {
            let new_h: u32 = text
                .lines
                .iter()
                .map(|l| line_visual_rows(l, content_width))
                .sum();
            // Empty text block (no lines) still occupies zero rows.
            let new_h = new_h.max(crate::cast::u32_sat(text.lines.len()));
            if new_h != visual_height.get() {
                visual_height.set(new_h);
                changed = true;
            }
        }
    }
    changed
}

/// Synchronise the `cell_height` of every `Mermaid` block in `blocks` with the
/// current cache. Call this before summing `total_lines` so scroll math reflects
/// whatever the cache knows at the time of the draw.
///
/// # Arguments
///
/// * `blocks`     – rendered document blocks for the active tab.
/// * `cache`      – mermaid render cache.
/// * `max_height` – upper bound in display lines (from `Config::mermaid_max_height`).
///
/// Returns `true` when at least one block's height changed, allowing callers to
/// skip expensive downstream work (like `recompute_positions`) when nothing moved.
pub fn update_mermaid_heights(
    blocks: &[DocBlock],
    cache: &crate::mermaid::MermaidCache,
    max_height: u32,
) -> bool {
    let mut changed = false;
    for block in blocks {
        if let DocBlock::Mermaid {
            id,
            source,
            cell_height,
            ..
        } = block
        {
            let new_h = cache.height(*id, source, max_height);
            if new_h != cell_height.get() {
                cell_height.set(new_h);
                changed = true;
            }
        }
    }
    changed
}

/// Return the source line for physical rendered row `local` inside a table,
/// consulting the cached `physical_to_source` mapping from `TableLayout`.
///
/// Falls back to the table's `source_line` when the cached layout is absent or
/// the index is out of range (race condition during first draw).
///
/// # Arguments
///
/// * `t`      – the `TableBlock` whose source lines are stored in
///   `row_source_lines`.
/// * `local`  – 0-based physical row index within the rendered table (same
///   coordinate space as `local_visual` in [`source_line_at_width`]).
/// * `layout` – optional cached layout from `MarkdownViewState::table_layouts`.
///   When `None` (before first draw), falls back to the old position-based math
///   using `row_source_lines` at fixed indices so behaviour is correct even
///   before wrapping data is available.
fn physical_row_source(
    t: &TableBlock,
    local: usize,
    layout: Option<&crate::ui::markdown_view::TableLayout>,
) -> u32 {
    if let Some(l) = layout {
        // Use the pre-computed mapping built by layout_table.
        return l
            .physical_to_source
            .get(local)
            .copied()
            .unwrap_or(t.source_line);
    }

    // Fallback: no cached layout yet (before first draw). Use the fixed-index
    // approximation with the pre-wrap layout (1 header row, 1 separator, 1 body
    // row each). This matches the old `table_row_source_line` behaviour and is
    // correct for tables that haven't been wrapped yet.
    let header_source = t.row_source_lines.first().copied().unwrap_or(t.source_line);
    let first_body_idx: usize = 3; // top-border + header + separator
    let last_body_idx: usize = first_body_idx + t.rows.len();

    if local <= 2 {
        header_source
    } else if local < last_body_idx {
        let body_index = local - first_body_idx;
        t.row_source_lines
            .get(1 + body_index)
            .copied()
            .unwrap_or(t.source_line)
    } else {
        t.row_source_lines.last().copied().unwrap_or(t.source_line)
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
/// Convenience wrapper around [`source_line_at_width`] used by tests and
/// the doctest of [`logical_line_at_source`]. Defaults to `content_width
/// = 0`, which short-circuits to "no wrap" (1 row per logical line).
#[allow(dead_code)]
pub fn source_line_at(blocks: &[DocBlock], visual_row: u32) -> u32 {
    source_line_at_width(blocks, visual_row, 0, &std::collections::HashMap::new())
}

/// Map a visual row to its 0-indexed markdown source line.
///
/// `visual_row` is in the same coordinate space as `MarkdownViewState`'s
/// `cursor_line` and `scroll_offset` — visual rows after wrapping. For
/// `Text` blocks, this walks `text.lines` summing per-line visual rows
/// until the target row is reached, then looks up `source_lines[logical_idx]`.
/// `content_width = 0` short-circuits to "no wrap" (1 row per logical line),
/// matching the legacy pre-wrap behavior tests rely on.
///
/// `table_layouts` must be the `MarkdownViewState::table_layouts` cache; it is
/// used to look up `physical_to_source` for table blocks so wrapped-cell tables
/// map physical sub-rows back to the correct markdown source line. Pass an empty
/// `HashMap` (or use [`source_line_at`]) in contexts where the layout cache is
/// unavailable (tests, pre-draw initialization).
///
/// # Arguments
///
/// * `blocks`        – rendered document block list.
/// * `visual_row`    – absolute visual row to map (0-indexed, post-wrap).
/// * `content_width` – viewport width for Text block visual-row counting.
/// * `table_layouts` – cached table render output from `MarkdownViewState`.
pub fn source_line_at_width(
    blocks: &[DocBlock],
    visual_row: u32,
    content_width: u16,
    table_layouts: &std::collections::HashMap<TableBlockId, crate::ui::markdown_view::TableLayout>,
) -> u32 {
    use crate::ui::markdown_view::visual_rows::line_visual_rows;
    let mut offset = 0u32;
    for block in blocks {
        let h = block.height();
        if visual_row < offset + h {
            let local_visual = visual_row - offset;
            return match block {
                DocBlock::Text {
                    text, source_lines, ..
                } => {
                    // Walk lines until accumulated visual rows reach
                    // `local_visual`; that logical index has the matching
                    // source line. With `content_width == 0` every line is
                    // 1 row so this collapses to direct indexing.
                    let mut acc = 0u32;
                    let mut logical_idx = 0usize;
                    for (i, line) in text.lines.iter().enumerate() {
                        let rows = line_visual_rows(line, content_width);
                        if local_visual < acc + rows {
                            logical_idx = i;
                            break;
                        }
                        acc += rows;
                        logical_idx = i + 1;
                    }
                    source_lines.get(logical_idx).copied().unwrap_or(0)
                }
                DocBlock::Mermaid {
                    source_line,
                    source,
                    ..
                } => {
                    // Mermaid blocks store their height as visual rows
                    // already (`cell_height`); within the block, each row
                    // corresponds 1:1 to a source line (no wrapping happens).
                    let local = local_visual as usize;
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
                        let content_count = crate::cast::u32_sat(source.lines().count());
                        let content_offset =
                            (crate::cast::u32_sat(local) - 1).min(content_count.saturating_sub(1));
                        *source_line + 1 + content_offset
                    }
                }
                DocBlock::Table(t) => {
                    let layout = table_layouts.get(&t.id);
                    physical_row_source(t, local_visual as usize, layout)
                }
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
/// Convenience wrapper around [`logical_line_at_source_width`] used by
/// tests. Defaults to `content_width = 0`.
#[allow(dead_code)]
pub fn logical_line_at_source(blocks: &[DocBlock], target_source: u32) -> Option<u32> {
    logical_line_at_source_width(blocks, target_source, 0)
}

/// Returns a position in *visual rows* — same coordinate space as
/// `MarkdownViewState::cursor_line` and `scroll_offset`. For Text blocks
/// this walks `text.lines` summing per-line visual rows, so the returned
/// index lands on the first wrapped row of the matching logical line.
/// `content_width = 0` short-circuits to "no wrap", matching the legacy
/// pre-wrap behavior tests rely on.
pub fn logical_line_at_source_width(
    blocks: &[DocBlock],
    target_source: u32,
    content_width: u16,
) -> Option<u32> {
    use crate::ui::markdown_view::visual_rows::line_visual_rows;
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
            DocBlock::Text {
                text, source_lines, ..
            } => {
                // Track visual-row offset of each logical line so the
                // position we return reflects where the line *actually
                // lands on screen*, not a stale logical index.
                let mut visual_in_block = 0u32;
                for (i, &s) in source_lines.iter().enumerate() {
                    if s == target_source {
                        return Some(offset + visual_in_block);
                    }
                    if s <= target_source {
                        best = Some(offset + visual_in_block);
                    }
                    if let Some(line) = text.lines.get(i) {
                        visual_in_block =
                            visual_in_block.saturating_add(line_visual_rows(line, content_width));
                    }
                }
            }
            DocBlock::Mermaid {
                source_line,
                source,
                ..
            } => {
                let content_count = crate::cast::u32_sat(source.lines().count());
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
                        3 + crate::cast::u32_sat(row_idx - 1) // body rows start at rendered index 3
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
    // Keep alphanumeric (any script), hyphens, underscores, and spaces;
    // drop everything else. GitHub's slugifier preserves `_` (it's not
    // alphanumeric per Unicode), so headings like `### \`foo_bar\`` slug
    // to `foo_bar` not `foobar`. TOC links of the form
    // `[\`foo_bar\`](#foo_bar)` rely on this.
    //
    // Consecutive hyphens are PRESERVED, not collapsed: GitHub's slug
    // for `# A / B` is `a--b` (the `/` drops, leaving the surrounding
    // spaces to each become `-`). TOC links with double-hyphens rely on
    // this.
    let filtered: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect();
    let slug = filtered.replace(' ', "-");
    // Strip leading/trailing hyphens that may appear after filtering.
    slug.trim_matches('-').to_string()
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

    /// GitHub's slugifier preserves consecutive hyphens (each space
    /// becomes its own hyphen, runs are NOT collapsed). TOC links of
    /// the form `[A / B](#a--b)` rely on this — the slash drops while
    /// each surrounding space contributes one hyphen.
    #[test]
    fn anchor_consecutive_spaces_preserve_hyphens() {
        assert_eq!(heading_to_anchor("A  B"), "a--b");
        assert_eq!(heading_to_anchor("A / B"), "a--b");
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

    /// Inline code (`` `text` ``) inside a heading must contribute to the
    /// slug. Without this the anchor is empty and TOC links of the form
    /// `[`kg.nodes`](#kgnodes)` silently fail to resolve, dropping out of
    /// the link picker.
    #[test]
    fn heading_with_inline_code_produces_correct_anchor() {
        let md = "# `kg.nodes`\n\nsome text\n";
        let blocks = render_markdown(md, &palette(), theme());
        let anchor = match &blocks[0] {
            DocBlock::Text {
                heading_anchors, ..
            } => heading_anchors.first().expect("anchor expected"),
            _ => panic!("expected Text block"),
        };
        // The dot is stripped by the slugifier, mirroring GitHub's anchor
        // for a heading like `### \`kg.nodes\``.
        assert_eq!(anchor.anchor, "kgnodes");
    }

    #[test]
    fn heading_mixing_text_and_inline_code_includes_both_in_anchor() {
        let md = "# Use the `Foo` API\n\nsome text\n";
        let blocks = render_markdown(md, &palette(), theme());
        let anchor = match &blocks[0] {
            DocBlock::Text {
                heading_anchors, ..
            } => heading_anchors.first().expect("anchor expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(anchor.anchor, "use-the-foo-api");
    }

    /// Underscores must survive slugification — GitHub-style anchors
    /// preserve them (`[\`foo_bar\`](#foo_bar)` is a common pattern in
    /// docs that link to inline-code headings).
    #[test]
    fn heading_with_underscores_preserves_underscores_in_anchor() {
        let md = "# `kg.node_stats`\n\nsome text\n";
        let blocks = render_markdown(md, &palette(), theme());
        let anchor = match &blocks[0] {
            DocBlock::Text {
                heading_anchors, ..
            } => heading_anchors.first().expect("anchor expected"),
            _ => panic!("expected Text block"),
        };
        assert_eq!(anchor.anchor, "kgnode_stats");
    }

    /// Multi-code heading separated by `/` should produce the GitHub
    /// slug with double hyphens around the slash + underscores
    /// preserved inside each code chunk.
    #[test]
    fn heading_with_multi_code_and_slash_produces_correct_anchor() {
        let md = "# `kg.node_stats` / `kg.predicate_stats`\n\nsome text\n";
        let blocks = render_markdown(md, &palette(), theme());
        let anchor = match &blocks[0] {
            DocBlock::Text {
                heading_anchors, ..
            } => heading_anchors.first().expect("anchor expected"),
            _ => panic!("expected Text block"),
        };
        // GitHub: spaces stay as `-`, slash drops, so `a / b` slug is `a--b`.
        assert_eq!(anchor.anchor, "kgnode_stats--kgpredicate_stats");
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
                            let text_after_prefix =
                                content.split_once(prefix).map_or("", |(_, t)| t).trim();
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
        let n = crate::cast::u32_sat(lines.len());
        DocBlock::Text {
            text: ratatui::text::Text::from(lines),
            links: vec![],
            heading_anchors: vec![],
            source_lines: sources.to_vec(),
            visual_height: std::cell::Cell::new(n),
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
        let b2 = text_block_with_sources(&["other", "dip-artifact"], &[310, 306]);
        // b1 height=1, b2 starts at offset 1. Target 306 is in b1 at 0.
        assert_eq!(logical_line_at_source(&[b1, b2], 306), Some(0));
    }

    #[test]
    fn logical_line_at_source_target_beyond_any_block_is_none() {
        // With no blocks at all there is no candidate anywhere, so None.
        assert_eq!(logical_line_at_source(&[], 5), None);
    }

    #[test]
    fn update_text_visual_heights_counts_wrapped_rows() {
        // A long line that wraps to multiple visual rows must have its
        // block height bumped accordingly so total_lines / scroll math
        // accounts for the extra rows. Without this, scrolling past a
        // wrapped paragraph leaves the next block visually shifted.
        let long = "x".repeat(50); // 50 chars wide
        let blocks = vec![text_block_with_sources(
            &["short", &long, "short"],
            &[0, 1, 2],
        )];
        // Width 20: long line wraps to ceil(50/20) = 3 rows.
        let changed = update_text_visual_heights(&blocks, 20);
        assert!(
            changed,
            "visual_height should change from logical (3) to visual (5)"
        );
        assert_eq!(blocks[0].height(), 5, "1 + 3 + 1");

        // Re-running with the same width does not change anything.
        let changed_again = update_text_visual_heights(&blocks, 20);
        assert!(!changed_again, "no-op on second call");

        // Wider width fits the long line on one row again.
        let changed_wider = update_text_visual_heights(&blocks, 80);
        assert!(changed_wider);
        assert_eq!(blocks[0].height(), 3, "1 + 1 + 1 at 80 cols");
    }

    #[test]
    fn source_line_at_width_handles_wrapped_text_block() {
        // With wrap, visual rows > logical lines. A query for visual row 2
        // should land on the source line of logical line 1 (the wrapped
        // line), not logical line 2.
        let long = "y".repeat(30);
        let blocks = vec![text_block_with_sources(&["a", &long, "c"], &[10, 20, 30])];
        // Width 10: long wraps to 3 rows. Visual layout:
        //   row 0: "a"           (logical 0, source 10)
        //   row 1: "yyyyyyyyyy"  (logical 1, source 20)
        //   row 2: "yyyyyyyyyy"  (logical 1, source 20)
        //   row 3: "yyyyyyyyyy"  (logical 1, source 20)
        //   row 4: "c"           (logical 2, source 30)
        update_text_visual_heights(&blocks, 10);
        let no_layouts = std::collections::HashMap::new();
        assert_eq!(source_line_at_width(&blocks, 0, 10, &no_layouts), 10);
        assert_eq!(source_line_at_width(&blocks, 1, 10, &no_layouts), 20);
        assert_eq!(
            source_line_at_width(&blocks, 2, 10, &no_layouts),
            20,
            "row 2 is wrap continuation"
        );
        assert_eq!(
            source_line_at_width(&blocks, 3, 10, &no_layouts),
            20,
            "row 3 is wrap continuation"
        );
        assert_eq!(source_line_at_width(&blocks, 4, 10, &no_layouts), 30);
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
