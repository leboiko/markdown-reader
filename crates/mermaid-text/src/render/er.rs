//! Renderer for [`ErDiagram`] (entity-relationship diagrams).
//!
//! **Phase 1** (this version): single-row source-order layout. Each
//! entity renders as a small box containing just its name; each
//! relationship draws as a horizontal arrow below the row, labelled
//! with `from-cardinality / to-cardinality` and the user-supplied
//! relationship label.
//!
//! Phase 2 (planned) replaces the name-only boxes with full attribute
//! tables and swaps the plain `→` arrow for proper cardinality
//! glyphs at each line endpoint. Phase 3 replaces the single-row
//! layout with a grid for diagrams that have more than ~4 entities.
//!
//! See `docs/scope-er-adoption.md` (TODO) for the full plan.

use unicode_width::UnicodeWidthStr;

use crate::er::{Cardinality, ErDiagram, Relationship};

/// Cells of horizontal padding between adjacent entity boxes in the
/// single-row layout. Big enough that relationship arrows have room
/// to label themselves between boxes; small enough that the row
/// fits comfortably in a typical terminal.
const ENTITY_GAP: usize = 4;

/// Cells of padding inside the entity box on each side of the name.
const NAME_PAD: usize = 2;

pub fn render(chart: &ErDiagram, _max_width: Option<usize>) -> String {
    if chart.entities.is_empty() {
        return String::new();
    }

    // ---- Layout: single-row source-order placement -------------------
    //
    // Each entity occupies columns `[entity_start[i], entity_end[i]]`.
    // The header row is row 0, name row is row 1, footer row is row 2.
    let entity_widths: Vec<usize> = chart
        .entities
        .iter()
        .map(|e| name_box_width(&e.name))
        .collect();

    let mut entity_left: Vec<usize> = Vec::with_capacity(chart.entities.len());
    {
        let mut col = 0usize;
        for &w in &entity_widths {
            entity_left.push(col);
            col += w + ENTITY_GAP;
        }
    }

    // Canvas width = rightmost entity's right edge + 1; height =
    // box (3 rows) + 2 spacer rows + N rows for relationships
    // (each takes 2 rows: arrow + label).
    let last_right = entity_left
        .last()
        .copied()
        .map(|left| left + entity_widths[entity_widths.len() - 1])
        .unwrap_or(0);
    let body_rows = if chart.relationships.is_empty() {
        1 // just the spacer below the row
    } else {
        2 + chart.relationships.len() * 2
    };
    let height = ENTITY_BOX_HEIGHT + body_rows;
    let width = last_right.max(1);

    let mut grid: Vec<Vec<char>> = vec![vec![' '; width]; height];

    // ---- 1. Draw entity boxes (header + name + footer) ----------------
    for (i, entity) in chart.entities.iter().enumerate() {
        let left = entity_left[i];
        let right = left + entity_widths[i] - 1;
        draw_entity_box(&mut grid, left, right, &entity.name);
    }

    // ---- 2. Draw relationships as horizontal arrows below the row ----
    //
    // Each relationship gets two consecutive rows: a label row above
    // the arrow row. They start from row `ENTITY_BOX_HEIGHT + 1`
    // (one row of breathing space below the boxes).
    for (rel_idx, rel) in chart.relationships.iter().enumerate() {
        let from_idx = chart.entity_index(&rel.from);
        let to_idx = chart.entity_index(&rel.to);
        let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else {
            continue;
        };
        let row_label = ENTITY_BOX_HEIGHT + 1 + rel_idx * 2;
        let row_arrow = row_label + 1;
        draw_relationship_line(
            &mut grid,
            entity_left[from_idx],
            entity_widths[from_idx],
            entity_left[to_idx],
            entity_widths[to_idx],
            row_label,
            row_arrow,
            rel,
        );
    }

    grid_to_string(&grid)
}

const ENTITY_BOX_HEIGHT: usize = 3;

/// Compute the width of an entity's name-box in cells: the name's
/// display width plus padding on each side plus the two border cells.
fn name_box_width(name: &str) -> usize {
    name.width() + 2 * NAME_PAD + 2
}

/// Draw a 3-row entity box (top border + centred name + bottom border)
/// with corners at `(left, 0)` and `(right, 2)`.
fn draw_entity_box(grid: &mut [Vec<char>], left: usize, right: usize, name: &str) {
    // Top border.
    put(grid, 0, left, '┌');
    for c in (left + 1)..right {
        put(grid, 0, c, '─');
    }
    put(grid, 0, right, '┐');

    // Name row — centre the name within the box's interior.
    put(grid, 1, left, '│');
    let interior_w = right - left - 1;
    let name_w = name.width();
    let name_start = left + 1 + (interior_w.saturating_sub(name_w)) / 2;
    put_str(grid, 1, name_start, name);
    put(grid, 1, right, '│');

    // Bottom border.
    put(grid, 2, left, '└');
    for c in (left + 1)..right {
        put(grid, 2, c, '─');
    }
    put(grid, 2, right, '┘');
}

/// Draw a single relationship line below the entity row. Phase 1 uses
/// a plain `→` arrowhead; Phase 2 will swap in proper cardinality
/// glyphs at each end.
#[allow(clippy::too_many_arguments)]
fn draw_relationship_line(
    grid: &mut [Vec<char>],
    from_left: usize,
    from_width: usize,
    to_left: usize,
    to_width: usize,
    row_label: usize,
    row_arrow: usize,
    rel: &Relationship,
) {
    let from_right_edge = from_left + from_width;
    let to_left_edge = to_left;

    // Determine arrow direction for the visible glyph at the target end.
    // (Source-to-target is always forward; if source is to the right
    // of target in source order, draw a left-pointing arrow.)
    let going_right = from_right_edge < to_left_edge;
    let (lo, hi) = if going_right {
        (from_right_edge, to_left_edge.saturating_sub(1))
    } else {
        (to_left + to_width, from_left.saturating_sub(1))
    };
    if hi <= lo {
        return; // adjacent entities with no room for an arrow
    }

    let line_glyph = if rel.line_style.is_dashed() { '┄' } else { '─' };
    let tip_glyph = if going_right { '▸' } else { '◂' };

    // Draw the line itself.
    for c in lo..=hi {
        put(grid, row_arrow, c, line_glyph);
    }
    // Tip on the target end.
    if going_right {
        put(grid, row_arrow, hi, tip_glyph);
    } else {
        put(grid, row_arrow, lo, tip_glyph);
    }

    // Label: cardinality summary + optional user label, placed above
    // the arrow starting just after the source entity.
    let summary = relationship_label_text(rel);
    if !summary.is_empty() {
        let label_col = lo + 1;
        put_str(grid, row_label, label_col, &summary);
    }
}

/// One-line summary text for a relationship: cardinality codes plus
/// the optional user label. The dashed-line vs solid-line style
/// already communicates identifying-vs-non-identifying visually, so
/// we don't add verbose text hints. Phase 2 will replace this
/// `from-card:to-card` notation with proper crow's-foot glyphs at
/// the line endpoints.
fn relationship_label_text(rel: &Relationship) -> String {
    let cards = format!(
        "{}:{}",
        cardinality_short(rel.from_cardinality),
        cardinality_short(rel.to_cardinality)
    );
    match &rel.label {
        Some(label) => format!("{cards} {label}"),
        None => cards,
    }
}

fn cardinality_short(c: Cardinality) -> &'static str {
    match c {
        Cardinality::ExactlyOne => "1",
        Cardinality::ZeroOrOne => "0..1",
        Cardinality::OneOrMany => "1..N",
        Cardinality::ZeroOrMany => "0..N",
    }
}

// ---------------------------------------------------------------------------
// Tiny grid helpers
// ---------------------------------------------------------------------------

fn put(grid: &mut [Vec<char>], row: usize, col: usize, ch: char) {
    if let Some(line) = grid.get_mut(row)
        && let Some(cell) = line.get_mut(col)
    {
        *cell = ch;
    }
}

fn put_str(grid: &mut [Vec<char>], row: usize, col: usize, s: &str) {
    let mut c = col;
    for ch in s.chars() {
        put(grid, row, c, ch);
        c += 1;
    }
}

fn grid_to_string(grid: &[Vec<char>]) -> String {
    let mut out = String::with_capacity(grid.iter().map(|r| r.len() + 1).sum());
    for row in grid {
        let line: String = row.iter().collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::er::parse;

    #[test]
    fn renders_two_entities_with_relationship() {
        let chart = parse("erDiagram\nCUSTOMER ||--o{ ORDER : places").unwrap();
        let out = render(&chart, None);
        assert!(out.contains("CUSTOMER"));
        assert!(out.contains("ORDER"));
        assert!(out.contains("▸"), "missing arrow tip in:\n{out}");
        assert!(out.contains("places"));
        // Cardinality summary should appear somewhere.
        assert!(out.contains("1:0..N"), "missing 1:0..N summary in:\n{out}");
    }

    #[test]
    fn renders_isolated_entity() {
        let chart = parse("erDiagram\nCUSTOMER {\n  string name\n}").unwrap();
        let out = render(&chart, None);
        assert!(out.contains("CUSTOMER"));
        // No relationship → no arrow.
        assert!(!out.contains("▸"));
    }

    #[test]
    fn renders_dashed_line_for_non_identifying() {
        let chart = parse("erDiagram\nA ||..o{ B").unwrap();
        let out = render(&chart, None);
        // Dashed line is the visual marker for non-identifying
        // relationships; the glyph itself communicates the style
        // (no verbose "(optional)" text hint needed).
        assert!(out.contains("┄"), "expected dashed line in:\n{out}");
        assert!(!out.contains("─▸"), "solid tip should not appear for dashed rel");
    }
}
