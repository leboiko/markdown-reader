//! Renderer for [`BlockDiagram`]. Produces a fixed-width grid of Unicode boxes
//! with edges drawn as inline arrow glyphs in the gaps between adjacent blocks.
//!
//! ## Layout
//!
//! Blocks are laid out in a grid. Each column is sized to the widest block
//! label that falls into it (a spanning block's label is spread across its
//! columns). The grid is rendered with Unicode box-drawing corners and
//! horizontal/vertical rules.
//!
//! Example for `columns 3`, blocks A, B (span 2), C:
//!
//! ```text
//! ┌───┐ ┌───────┐ ┌───┐
//! │ A │ │   B   │ │ C │
//! └───┘ └───────┘ └───┘
//! ```
//!
//! ## Edge rendering strategy (0.42.0)
//!
//! Edges between **horizontally-adjacent** blocks (same row, neighbouring
//! columns) are drawn as `►` (forward) or `◄` (reverse) in the single-character
//! column gap on the content line between the two boxes.
//!
//! Edges between **vertically-adjacent** blocks (same column, neighbouring
//! rows) are drawn as `▼` (forward) or `▲` (reverse) in the blank separator
//! row between the two row groups, at the horizontal position of the shared
//! column's content centre.
//!
//! Edges that are neither horizontally nor vertically adjacent fall back to a
//! short text summary appended below the grid (Tier 3). The "Edges:" header is
//! omitted entirely when all edges are routed inline.
//!
//! ## max_width
//!
//! When `max_width` is `Some(n)`, block label text is truncated with `…` so
//! that the total grid width does not exceed the budget. The fallback edge
//! summary is not truncated.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::block_diagram::{Block, BlockDiagram, BlockEdge};

/// Number of spaces between adjacent column boxes.
const COL_GAP: usize = 1;

/// Minimum inner width (characters) for a single column cell.
const MIN_CELL_INNER: usize = 1;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render a [`BlockDiagram`] to a Unicode string.
///
/// # Arguments
///
/// * `diag`      — the parsed diagram
/// * `max_width` — optional column budget; block labels are truncated with `…`
///   when the natural grid exceeds this budget
///
/// # Returns
///
/// A multi-line string ready for printing. The grid uses Unicode box-drawing
/// characters (`┌ ─ ┐ │ └ ┘`) with blocks separated by single-space gaps.
/// Spanning blocks merge their column widths and the gap between them.
/// Adjacent directed edges are drawn inline as arrow glyphs; non-adjacent edges
/// fall back to a short text summary below the grid.
pub fn render(diag: &BlockDiagram, max_width: Option<usize>) -> String {
    if diag.blocks.is_empty() {
        let unrouted: Vec<&BlockEdge> = diag.edges.iter().collect();
        return render_edge_summary(&unrouted);
    }

    let cols = diag.columns.max(1);
    let placements = compute_placements(&diag.blocks, cols);
    let col_inner_widths = compute_col_widths(&diag.blocks, &placements, cols);
    let col_inner_widths = apply_max_width(col_inner_widths, max_width, cols);
    let block_pos_map = build_block_pos_map(diag, &placements);
    let row_count = placements.iter().map(|p| p.row + 1).max().unwrap_or(0);

    let mut out = String::new();

    for row in 0..row_count {
        let row_slots = collect_row_slots(&placements, &diag.blocks, row);

        out.push_str(&build_top_line(&row_slots, &col_inner_widths));
        out.push('\n');

        let h_arrows = collect_horizontal_arrows(diag, &block_pos_map, &row_slots, &col_inner_widths, row);
        out.push_str(&build_content_line(&row_slots, &col_inner_widths, &h_arrows));
        out.push('\n');

        out.push_str(&build_bottom_line(&row_slots, &col_inner_widths));
        out.push('\n');

        if row + 1 < row_count {
            let v_arrows = collect_vertical_arrows(diag, &block_pos_map, &row_slots, &col_inner_widths, row);
            let gap = build_gap_line(&v_arrows);
            out.push_str(&gap);
            out.push('\n');
        }
    }

    let unrouted = collect_unrouted_edges(diag, &block_pos_map);
    if !unrouted.is_empty() {
        out.push('\n');
        out.push_str(&render_edge_summary(&unrouted));
    }

    while out.ends_with('\n') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Placement {
    block_idx: usize,
    row: usize,
    col_start: usize,
    col_end: usize,
}

fn compute_placements(blocks: &[Block], cols: usize) -> Vec<Placement> {
    let mut placements = Vec::with_capacity(blocks.len());
    let mut row = 0usize;
    let mut col = 0usize;

    for (idx, block) in blocks.iter().enumerate() {
        let span = block.col_span.min(cols).max(1);
        if col + span > cols && col > 0 {
            row += 1;
            col = 0;
        }
        placements.push(Placement { block_idx: idx, row, col_start: col, col_end: col + span });
        col += span;
        if col >= cols {
            row += 1;
            col = 0;
        }
    }
    placements
}

// ---------------------------------------------------------------------------
// Column width computation
// ---------------------------------------------------------------------------

fn compute_col_widths(blocks: &[Block], placements: &[Placement], cols: usize) -> Vec<usize> {
    let mut col_widths = vec![MIN_CELL_INNER; cols];
    for p in placements {
        let block = &blocks[p.block_idx];
        let lw = UnicodeWidthStr::width(block.display_text());
        let span = p.col_end - p.col_start;
        if span == 1 {
            col_widths[p.col_start] = col_widths[p.col_start].max(lw);
        } else {
            let gap_absorbed = (span - 1) * (COL_GAP + 2);
            let needed = lw.saturating_sub(gap_absorbed).div_ceil(span).max(MIN_CELL_INNER);
            for w in col_widths.iter_mut().take(p.col_end).skip(p.col_start) {
                *w = (*w).max(needed);
            }
        }
    }
    col_widths
}

fn apply_max_width(mut col_widths: Vec<usize>, max_width: Option<usize>, cols: usize) -> Vec<usize> {
    let Some(budget) = max_width else { return col_widths; };
    if grid_natural_width(&col_widths, cols) <= budget {
        return col_widths;
    }
    for _ in 0..100 {
        if grid_natural_width(&col_widths, cols) <= budget {
            break;
        }
        let max_w = *col_widths.iter().max().unwrap_or(&MIN_CELL_INNER);
        if max_w <= MIN_CELL_INNER {
            break;
        }
        for w in &mut col_widths {
            if *w == max_w {
                *w -= 1;
                break;
            }
        }
    }
    col_widths
}

fn grid_natural_width(col_widths: &[usize], cols: usize) -> usize {
    if cols == 0 { return 0; }
    col_widths.iter().take(cols).sum::<usize>() + cols * 2 + (cols - 1) * COL_GAP
}

/// Inner width of a block that spans `col_start..col_end`, absorbing the
/// gap and wall characters between its columns.
fn spanned_inner_width(col_widths: &[usize], col_start: usize, col_end: usize) -> usize {
    let span = col_end - col_start;
    let base: usize = col_widths[col_start..col_end.min(col_widths.len())].iter().sum();
    if span <= 1 { base } else { base + (span - 1) * (COL_GAP + 2) }
}

fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w == 0 { return String::new(); }
    if UnicodeWidthStr::width(s) <= max_w { return s.to_string(); }
    let target = max_w.saturating_sub(1);
    let mut result = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + cw > target { break; }
        result.push(ch);
        used += cw;
    }
    result.push('\u{2026}');
    result
}

// ---------------------------------------------------------------------------
// Row slot — a block assigned to a specific row
// ---------------------------------------------------------------------------

/// One block's position within a rendered grid row.
struct RowSlot<'d> {
    col_start: usize,
    col_end: usize,
    block: &'d Block,
}

fn collect_row_slots<'d>(placements: &[Placement], blocks: &'d [Block], row: usize) -> Vec<RowSlot<'d>> {
    let mut slots: Vec<RowSlot<'d>> = placements
        .iter()
        .filter(|p| p.row == row)
        .map(|p| RowSlot { col_start: p.col_start, col_end: p.col_end, block: &blocks[p.block_idx] })
        .collect();
    slots.sort_by_key(|s| s.col_start);
    slots
}

// ---------------------------------------------------------------------------
// Block position map for adjacency detection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct BlockGridPos {
    row: usize,
    col_start: usize,
    col_end: usize,
}

fn build_block_pos_map(diag: &BlockDiagram, placements: &[Placement]) -> HashMap<String, BlockGridPos> {
    placements
        .iter()
        .map(|p| {
            let id = diag.blocks[p.block_idx].id.clone();
            (id, BlockGridPos { row: p.row, col_start: p.col_start, col_end: p.col_end })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Adjacency detection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Adjacency {
    HorizontalForward,
    HorizontalReverse,
    VerticalForward,
    VerticalReverse,
    NotAdjacent,
}

fn detect_adjacency(src: BlockGridPos, tgt: BlockGridPos) -> Adjacency {
    if src.row == tgt.row {
        if src.col_end == tgt.col_start { return Adjacency::HorizontalForward; }
        if tgt.col_end == src.col_start { return Adjacency::HorizontalReverse; }
    }
    // Vertical adjacency: blocks must share at least one column in their spans.
    let cols_overlap = src.col_start < tgt.col_end && tgt.col_start < src.col_end;
    if cols_overlap {
        if src.row + 1 == tgt.row { return Adjacency::VerticalForward; }
        if tgt.row + 1 == src.row { return Adjacency::VerticalReverse; }
    }
    Adjacency::NotAdjacent
}

// ---------------------------------------------------------------------------
// Horizontal arrow injection
// ---------------------------------------------------------------------------

/// A glyph to place at character offset `gap_char_idx` in the content line.
struct HorizontalArrow {
    gap_char_idx: usize,
    glyph: char,
}

/// Walk the row slots left-to-right to compute actual character offsets for each
/// inter-box gap, then match edges to gaps.
fn collect_horizontal_arrows(
    diag: &BlockDiagram,
    block_pos_map: &HashMap<String, BlockGridPos>,
    row_slots: &[RowSlot<'_>],
    col_inner_widths: &[usize],
    row: usize,
) -> Vec<HorizontalArrow> {
    // Build a map: (col_end_of_left_block, col_start_of_right_block) -> char_idx_of_gap
    // by walking slots and accumulating character widths.
    let gap_map = build_gap_char_map(row_slots, col_inner_widths);

    let mut arrows = Vec::new();
    for edge in &diag.edges {
        let Some(src) = block_pos_map.get(&edge.source) else { continue; };
        let Some(tgt) = block_pos_map.get(&edge.target) else { continue; };
        if src.row != row || tgt.row != row { continue; }
        match detect_adjacency(*src, *tgt) {
            Adjacency::HorizontalForward => {
                if let Some(idx) = gap_map.get(&(src.col_end, tgt.col_start)) {
                    arrows.push(HorizontalArrow { gap_char_idx: *idx, glyph: '\u{25BA}' }); // ►
                }
            }
            Adjacency::HorizontalReverse => {
                if let Some(idx) = gap_map.get(&(tgt.col_end, src.col_start)) {
                    arrows.push(HorizontalArrow { gap_char_idx: *idx, glyph: '\u{25C4}' }); // ◄
                }
            }
            _ => {}
        }
    }
    arrows
}

/// Build a map from (col_end_of_left, col_start_of_right) to the 0-based
/// character index of the gap character (the single space between boxes).
fn build_gap_char_map(
    row_slots: &[RowSlot<'_>],
    col_inner_widths: &[usize],
) -> HashMap<(usize, usize), usize> {
    let mut map = HashMap::new();
    let mut char_x = 0usize;
    for (i, slot) in row_slots.iter().enumerate() {
        let inner_w = spanned_inner_width(col_inner_widths, slot.col_start, slot.col_end);
        // Box renders as: │(1) space(1) label(inner_w) space(1) │(1) = inner_w + 4 chars
        let box_char_width = inner_w + 4;
        if i + 1 < row_slots.len() {
            let gap_char_idx = char_x + box_char_width; // first char after closing │
            let next = &row_slots[i + 1];
            map.insert((slot.col_end, next.col_start), gap_char_idx);
        }
        char_x += box_char_width + COL_GAP;
    }
    map
}

// ---------------------------------------------------------------------------
// Vertical arrow injection
// ---------------------------------------------------------------------------

/// A glyph to place at character offset `centre_x` in the gap line.
struct VerticalArrow {
    centre_x: usize,
    glyph: char,
}

fn collect_vertical_arrows(
    diag: &BlockDiagram,
    block_pos_map: &HashMap<String, BlockGridPos>,
    upper_row_slots: &[RowSlot<'_>],
    col_inner_widths: &[usize],
    upper_row: usize,
) -> Vec<VerticalArrow> {
    // Precompute the content-centre character offset for each slot in the upper row.
    let centre_map = build_slot_centre_map(upper_row_slots, col_inner_widths);

    let mut arrows = Vec::new();
    for edge in &diag.edges {
        let Some(src) = block_pos_map.get(&edge.source) else { continue; };
        let Some(tgt) = block_pos_map.get(&edge.target) else { continue; };
        let (above, glyph) = match detect_adjacency(*src, *tgt) {
            Adjacency::VerticalForward  => (*src, '\u{25BC}'), // ▼
            Adjacency::VerticalReverse  => (*tgt, '\u{25B2}'), // ▲
            _ => continue,
        };
        if above.row != upper_row { continue; }
        if let Some(centre_x) = centre_map.get(&(above.col_start, above.col_end)) {
            arrows.push(VerticalArrow { centre_x: *centre_x, glyph });
        }
    }
    arrows
}

/// Map from (col_start, col_end) of a slot to the character-column of its
/// content midpoint, measured from the left edge of the row.
fn build_slot_centre_map(
    row_slots: &[RowSlot<'_>],
    col_inner_widths: &[usize],
) -> HashMap<(usize, usize), usize> {
    let mut map = HashMap::new();
    let mut char_x = 0usize;
    for slot in row_slots {
        let inner_w = spanned_inner_width(col_inner_widths, slot.col_start, slot.col_end);
        // Content centre: after │(1) space(1), then inner_w/2 into the label area.
        let centre = char_x + 1 + 1 + inner_w / 2;
        map.insert((slot.col_start, slot.col_end), centre);
        char_x += inner_w + 4 + COL_GAP;
    }
    map
}

// ---------------------------------------------------------------------------
// Grid line builders
// ---------------------------------------------------------------------------

fn build_top_line(row_slots: &[RowSlot<'_>], col_inner_widths: &[usize]) -> String {
    let mut line = String::new();
    let mut col_cursor = 0usize;
    for slot in row_slots {
        if slot.col_start > col_cursor {
            let gap = (slot.col_start - col_cursor) * (MIN_CELL_INNER + 2 + COL_GAP);
            for _ in 0..gap { line.push(' '); }
        }
        let inner_w = spanned_inner_width(col_inner_widths, slot.col_start, slot.col_end);
        line.push('\u{250C}'); // ┌
        for _ in 0..inner_w + 2 { line.push('\u{2500}'); } // ─
        line.push('\u{2510}'); // ┐
        col_cursor = slot.col_end;
        for _ in 0..COL_GAP { line.push(' '); }
    }
    line.trim_end().to_string()
}

fn build_bottom_line(row_slots: &[RowSlot<'_>], col_inner_widths: &[usize]) -> String {
    let mut line = String::new();
    let mut col_cursor = 0usize;
    for slot in row_slots {
        if slot.col_start > col_cursor {
            let gap = (slot.col_start - col_cursor) * (MIN_CELL_INNER + 2 + COL_GAP);
            for _ in 0..gap { line.push(' '); }
        }
        let inner_w = spanned_inner_width(col_inner_widths, slot.col_start, slot.col_end);
        line.push('\u{2514}'); // └
        for _ in 0..inner_w + 2 { line.push('\u{2500}'); } // ─
        line.push('\u{2518}'); // ┘
        col_cursor = slot.col_end;
        for _ in 0..COL_GAP { line.push(' '); }
    }
    line.trim_end().to_string()
}

fn build_content_line(
    row_slots: &[RowSlot<'_>],
    col_inner_widths: &[usize],
    h_arrows: &[HorizontalArrow],
) -> String {
    let mut buf = String::new();
    let mut col_cursor = 0usize;

    for slot in row_slots {
        if slot.col_start > col_cursor {
            let gap = (slot.col_start - col_cursor) * (MIN_CELL_INNER + 2 + COL_GAP);
            for _ in 0..gap { buf.push(' '); }
        }
        let inner_w = spanned_inner_width(col_inner_widths, slot.col_start, slot.col_end);
        let label = slot.block.display_text();
        let label_w = UnicodeWidthStr::width(label);
        let label = if label_w > inner_w {
            truncate_to_width(label, inner_w)
        } else {
            label.to_string()
        };
        let label_w = UnicodeWidthStr::width(label.as_str());
        let total_pad = inner_w.saturating_sub(label_w);
        let left_pad = total_pad / 2;
        let right_pad = total_pad - left_pad;

        buf.push('\u{2502}'); // │
        buf.push(' ');
        for _ in 0..left_pad { buf.push(' '); }
        buf.push_str(&label);
        for _ in 0..right_pad { buf.push(' '); }
        buf.push(' ');
        buf.push('\u{2502}'); // │

        col_cursor = slot.col_end;
        for _ in 0..COL_GAP { buf.push(' '); }
    }

    // Stamp horizontal arrows into the gap positions (mutate char vector).
    if h_arrows.is_empty() {
        return buf.trim_end().to_string();
    }

    let mut chars: Vec<char> = buf.chars().collect();
    for arrow in h_arrows {
        if arrow.gap_char_idx < chars.len() {
            chars[arrow.gap_char_idx] = arrow.glyph;
        }
    }
    while chars.last() == Some(&' ') { chars.pop(); }
    chars.into_iter().collect()
}

/// Build the gap separator line between two grid rows.
///
/// When no vertical edges cross this gap the line is empty (preserving the
/// existing blank-line separator behaviour). When vertical arrows exist we
/// stamp them at their computed character positions.
fn build_gap_line(v_arrows: &[VerticalArrow]) -> String {
    if v_arrows.is_empty() {
        return String::new();
    }
    let max_x = v_arrows.iter().map(|a| a.centre_x).max().unwrap_or(0);
    let mut chars: Vec<char> = vec![' '; max_x + 1];
    for arrow in v_arrows {
        chars[arrow.centre_x] = arrow.glyph;
    }
    while chars.last() == Some(&' ') { chars.pop(); }
    chars.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Unrouted edge collection (Tier 3 fallback)
// ---------------------------------------------------------------------------

fn collect_unrouted_edges<'a>(
    diag: &'a BlockDiagram,
    block_pos_map: &HashMap<String, BlockGridPos>,
) -> Vec<&'a BlockEdge> {
    diag.edges
        .iter()
        .filter(|e| {
            let Some(src) = block_pos_map.get(&e.source) else { return true; };
            let Some(tgt) = block_pos_map.get(&e.target) else { return true; };
            detect_adjacency(*src, *tgt) == Adjacency::NotAdjacent
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Edge summary (Tier 3 text fallback)
// ---------------------------------------------------------------------------

fn render_edge_summary(edges: &[&BlockEdge]) -> String {
    if edges.is_empty() { return String::new(); }
    let mut out = String::from("Edges:\n");
    for edge in edges {
        if let Some(label) = &edge.label {
            out.push_str(&format!(
                "  {} \u{2500}\u{2500}\u{25BA} {} [{}]\n",
                edge.source, edge.target, label
            ));
        } else {
            out.push_str(&format!(
                "  {} \u{2500}\u{2500}\u{25BA} {}\n",
                edge.source, edge.target
            ));
        }
    }
    while out.ends_with('\n') { out.pop(); }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::block_diagram::parse;

    fn parsed(src: &str) -> BlockDiagram {
        parse(src).expect("parse should succeed")
    }

    #[test]
    fn renders_single_block() {
        let diag = parsed("block-beta\n    A");
        let out = render(&diag, None);
        assert!(out.contains('A'), "block label 'A' must appear in output:\n{out}");
        assert!(out.contains('\u{250C}'), "top-left corner ┌ must appear:\n{out}");
        assert!(out.contains('\u{2518}'), "bottom-right corner ┘ must appear:\n{out}");
    }

    #[test]
    fn renders_blocks_with_text_labels() {
        let diag = parsed("block-beta\n    columns 2\n    a[\"Alpha\"] b[\"Beta\"]");
        let out = render(&diag, None);
        assert!(out.contains("Alpha"), "Alpha label missing:\n{out}");
        assert!(out.contains("Beta"), "Beta label missing:\n{out}");
    }

    #[test]
    fn horizontal_adjacent_edge_draws_inline_arrow() {
        let diag = parsed("block-beta\n    columns 2\n    A B\n    A --> B");
        let out = render(&diag, None);
        assert!(
            out.contains('\u{25BA}'),
            "inline ► arrow missing for horizontal adjacent edge:\n{out}"
        );
        assert!(
            !out.contains("Edges:"),
            "Edges: summary must be absent when edge is routed inline:\n{out}"
        );
    }

    #[test]
    fn vertical_adjacent_edge_draws_inline_arrow() {
        let diag = parsed("block-beta\n    columns 1\n    A\n    B\n    A --> B");
        let out = render(&diag, None);
        assert!(
            out.contains('\u{25BC}'),
            "inline ▼ arrow missing for vertical adjacent edge:\n{out}"
        );
        assert!(
            !out.contains("Edges:"),
            "Edges: summary must be absent when edge is routed inline:\n{out}"
        );
    }

    #[test]
    fn non_adjacent_edge_falls_back_to_summary() {
        let diag = parsed("block-beta\n    columns 3\n    A B C\n    D E F\n    A --> F");
        let out = render(&diag, None);
        assert!(
            out.contains("Edges:"),
            "non-adjacent edge must appear in Edges: summary:\n{out}"
        );
    }

    #[test]
    fn grid_integrity_preserved_with_edges() {
        let diag = parsed(
            "block-beta\n    columns 3\n    A B C\n    D E F\n    A --> B\n    D --> E",
        );
        let out = render(&diag, None);
        let corner_count: usize = out.chars().filter(|&c| c == '\u{250C}').count();
        assert_eq!(
            corner_count, 6,
            "expected 6 ┌ corners for 6 blocks, got {corner_count}:\n{out}"
        );
    }

    #[test]
    fn empty_diagram_renders_without_panic() {
        let diag = BlockDiagram::default();
        let out = render(&diag, None);
        assert!(!out.contains('\u{250C}'), "no box should be drawn for empty diagram");
    }

    #[test]
    fn max_width_truncates_long_labels() {
        let diag = parsed("block-beta\n    a[\"This is a very long label that overflows\"]");
        let out = render(&diag, Some(20));
        for line in out.lines() {
            let w = UnicodeWidthStr::width(line);
            assert!(w <= 22, "line width {w} exceeds budget: {line:?}");
        }
    }

    #[test]
    fn spanning_block_renders_wider_box() {
        let diag = parsed("block-beta\n    columns 3\n    a b:2 c\n    d e f");
        let out = render(&diag, None);
        for id in &["a", "b", "c", "d", "e", "f"] {
            assert!(out.contains(id), "block {id} missing from output:\n{out}");
        }
        let total_corners: usize = out
            .lines()
            .map(|l| l.chars().filter(|&c| c == '\u{250C}').count())
            .sum();
        assert!(
            total_corners >= 6,
            "expected ≥6 ┌ corners across all rows, got {total_corners}:\n{out}"
        );
        assert!(out.contains("b "), "b label with trailing space missing:\n{out}");
    }

    #[test]
    fn labelled_edge_adjacent_draws_inline_arrow() {
        // Adjacent blocks in same row — the edge is routed inline as ►.
        // Edge label is dropped (not rendered on the glyph — Tier 1 limitation).
        let diag = parsed("block-beta\n    columns 2\n    A B\n    A -->|calls| B");
        let out = render(&diag, None);
        assert!(out.contains('\u{25BA}'), "inline ► arrow missing:\n{out}");
    }

    #[test]
    fn multi_row_grid_has_separator_lines() {
        let diag = parsed("block-beta\n    columns 1\n    A\n    B\n    C");
        let out = render(&diag, None);
        // Three rows → two gap separators. A gap line is either empty or contains a ▼.
        let non_box_lines: Vec<&str> = out
            .lines()
            .filter(|l| !l.contains('\u{250C}') && !l.contains('\u{2502}') && !l.contains('\u{2514}'))
            .collect();
        assert!(
            non_box_lines.len() >= 2,
            "expected ≥2 separator lines between rows, got {}:\n{out}",
            non_box_lines.len()
        );
    }

    #[test]
    fn reverse_horizontal_edge_draws_left_arrow() {
        let diag = parsed("block-beta\n    columns 2\n    A B\n    B --> A");
        let out = render(&diag, None);
        assert!(
            out.contains('\u{25C4}'),
            "inline ◄ arrow missing for reverse horizontal edge:\n{out}"
        );
    }
}
