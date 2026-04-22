//! Renderer for [`ErDiagram`] (entity-relationship diagrams).
//!
//! **Phase 2.1** (this version): the relationship line now visually
//! connects the two entity boxes — drawn at the entity-name row,
//! with `┤` / `├` tee glyphs replacing the source/target side
//! borders, cardinality glyphs adjacent to each border, and the
//! label (when present) centred on a row above the boxes.
//!
//! Identifying relationships use solid `─` lines; non-identifying
//! use dashed `┄`. Entity boxes carry attribute tables (header +
//! divider + per-attribute rows) with cardinality glyphs (`1`
//! exactly one, `?` zero or one, `+` one or many, `*` zero or many).
//!
//! Phase 3 (planned) replaces the single-row layout with a grid for
//! diagrams with more than ~4 entities.

use unicode_width::UnicodeWidthStr;

use crate::er::{AttributeKey, Cardinality, ErDiagram, Relationship};

/// Minimum cells of horizontal padding between adjacent entity boxes
/// when no relationship runs between them. Just wide enough that
/// boxes don't visually merge.
const MIN_ENTITY_GAP: usize = 4;

/// Cells of padding inside the entity box on each side of content
/// (the entity name in the header, the type/name/keys columns in
/// attribute rows).
const NAME_PAD: usize = 2;

pub fn render(chart: &ErDiagram, _max_width: Option<usize>) -> String {
    if chart.entities.is_empty() {
        return String::new();
    }

    // Per-entity box geometry: box width (widest content column +
    // padding) and box height (3 header rows + 1 divider + per-
    // attribute rows).
    let entity_widths: Vec<usize> = chart.entities.iter().map(entity_box_width).collect();
    let entity_heights: Vec<usize> = chart.entities.iter().map(entity_box_height).collect();
    let tallest = *entity_heights.iter().max().unwrap_or(&HEADER_ROWS);

    // Per-pair gap: widen to fit the relationship label when one
    // exists between this entity and its left neighbour, plus the
    // two cardinality glyphs and one cell of breathing room each
    // side.
    let pair_gaps = compute_pair_gaps(chart);

    // Entities lay out left-to-right in source order with the per-
    // pair gap to the right of each (gap[i] sits between entity i
    // and entity i+1).
    let entity_left: Vec<usize> = {
        let mut out = Vec::with_capacity(chart.entities.len());
        let mut col = 0usize;
        for (i, &w) in entity_widths.iter().enumerate() {
            out.push(col);
            col += w + pair_gaps.get(i).copied().unwrap_or(MIN_ENTITY_GAP);
        }
        out
    };

    let last_right = entity_left
        .last()
        .zip(entity_widths.last())
        .map(|(&left, &w)| left + w)
        .unwrap_or(0);

    // The relationship line lives on the entity-name row of every
    // entity (always the second row of any entity box). When any
    // relationship has a label, reserve one row above the boxes so
    // the label has somewhere to sit without overwriting the top
    // border. Same for the source/target cardinality fallbacks
    // when the line can't merge with the box border (rare).
    let has_labels = chart
        .relationships
        .iter()
        .any(|r| r.label.as_deref().is_some_and(|s| !s.is_empty()));
    let top_pad: usize = if has_labels { 1 } else { 0 };

    // Canvas: optional top label row + tallest entity. Relationships
    // are now drawn ON the entity grid, not in extra rows below.
    let height = top_pad + tallest;
    let width = last_right.max(1);

    let mut grid: Vec<Vec<char>> = vec![vec![' '; width]; height];

    // Pass 1: draw each entity box (header + attribute rows).
    for (i, entity) in chart.entities.iter().enumerate() {
        let left = entity_left[i];
        let right = left + entity_widths[i] - 1;
        draw_entity_box(&mut grid, top_pad, left, right, entity);
    }

    // Pass 2: connect each relationship's two entities with a
    // visible line through the side borders. The line sits on the
    // entity-name row (always present in every box) and meets the
    // border via `┤` / `├` tee glyphs.
    for rel in &chart.relationships {
        let (Some(from_idx), Some(to_idx)) =
            (chart.entity_index(&rel.from), chart.entity_index(&rel.to))
        else {
            continue;
        };
        if from_idx == to_idx {
            // Self-relationships need a different visual (loop) —
            // skip for now; a future phase can route a self-loop
            // around the entity perimeter.
            continue;
        }
        draw_relationship_line(
            &mut grid,
            top_pad,
            entity_left[from_idx],
            entity_widths[from_idx],
            entity_left[to_idx],
            entity_widths[to_idx],
            rel,
        );
    }

    grid_to_string(&grid)
}

/// Compute the inter-entity gap for every adjacent pair (i, i+1).
/// `gaps[i]` is the gap between entity `i` and entity `i+1`, sized
/// to fit the widest relationship label that runs between them and
/// the cardinality glyphs at each end.
fn compute_pair_gaps(chart: &ErDiagram) -> Vec<usize> {
    let n = chart.entities.len();
    if n < 2 {
        return vec![MIN_ENTITY_GAP; n];
    }
    let mut gaps = vec![MIN_ENTITY_GAP; n];
    for rel in &chart.relationships {
        let Some(from_idx) = chart.entity_index(&rel.from) else {
            continue;
        };
        let Some(to_idx) = chart.entity_index(&rel.to) else {
            continue;
        };
        if from_idx == to_idx {
            continue;
        }
        let (lo_idx, hi_idx) = if from_idx <= to_idx {
            (from_idx, to_idx)
        } else {
            (to_idx, from_idx)
        };
        // Required gap = label width + 2 cardinality glyphs + 2 cells
        // of breathing room between the glyphs and the label/borders.
        let label_w = rel.label.as_deref().map(|s| s.width()).unwrap_or(0);
        let needed = label_w.max(2) + 4;
        // Distribute across consecutive pairs the relationship
        // spans (so a relationship between non-adjacent entities
        // widens every gap it crosses). For now we widen each
        // crossed pair to the same `needed` value — simple and
        // produces a clean line on the README two-entity case.
        for gap in gaps.iter_mut().take(hi_idx).skip(lo_idx) {
            *gap = (*gap).max(needed);
        }
    }
    gaps
}

/// Rows consumed by the entity-name header: top border + name +
/// divider. Attribute rows sit below. Bottom border is appended last
/// so the entity's total height is `HEADER_ROWS + attrs + 1`.
const HEADER_ROWS: usize = 3;

/// Widths of the three attribute-table columns inside an entity box:
/// `type`, `name`, `keys`. Computed per entity so every box is
/// snug — no wasted horizontal space for short-attribute tables.
struct AttrColumns {
    type_w: usize,
    name_w: usize,
    keys_w: usize,
}

/// Compute per-column widths across all attribute rows in an entity.
fn attr_columns(entity: &crate::er::Entity) -> AttrColumns {
    let mut cols = AttrColumns {
        type_w: 0,
        name_w: 0,
        keys_w: 0,
    };
    for attr in &entity.attributes {
        cols.type_w = cols.type_w.max(attr.type_name.width());
        cols.name_w = cols.name_w.max(attr.name.width());
        cols.keys_w = cols.keys_w.max(format_keys(&attr.keys).width());
    }
    cols
}

/// Total box width for an entity: the max of (header width, attribute
/// table width) plus padding and borders. If the entity has no
/// attributes, only the header counts.
fn entity_box_width(entity: &crate::er::Entity) -> usize {
    let header_w = entity.name.width() + 2 * NAME_PAD + 2;
    if entity.attributes.is_empty() {
        return header_w;
    }
    let cols = attr_columns(entity);
    // Attribute row = "  <type> <name> <keys>  │" → two padding
    // cells + col_widths + 2 spaces between columns + 2 border cells.
    let attr_w = 2 * NAME_PAD + cols.type_w + 1 + cols.name_w + 1 + cols.keys_w + 2;
    attr_w.max(header_w)
}

/// Total box height for an entity: header rows + one row per
/// attribute + bottom border. 4 when no attributes (header + 1 name
/// + bottom), else `3 + attributes + 1`.
fn entity_box_height(entity: &crate::er::Entity) -> usize {
    if entity.attributes.is_empty() {
        HEADER_ROWS
    } else {
        HEADER_ROWS + entity.attributes.len() + 1
    }
}

/// Draw the full entity box: top border + centred name + divider (if
/// attributes) + one row per attribute + bottom border. The box
/// starts at `top_pad`; rows above are reserved for relationship
/// labels.
fn draw_entity_box(
    grid: &mut [Vec<char>],
    top_pad: usize,
    left: usize,
    right: usize,
    entity: &crate::er::Entity,
) {
    let interior_w = right - left - 1;
    let name_w = entity.name.width();
    let name_start = left + 1 + (interior_w.saturating_sub(name_w)) / 2;

    // Top border.
    put(grid, top_pad, left, '┌');
    for c in (left + 1)..right {
        put(grid, top_pad, c, '─');
    }
    put(grid, top_pad, right, '┐');

    // Name row — centre horizontally.
    put(grid, top_pad + 1, left, '│');
    put_str(grid, top_pad + 1, name_start, &entity.name);
    put(grid, top_pad + 1, right, '│');

    if entity.attributes.is_empty() {
        // Bare entity — just a 3-row box (no divider, no rows).
        put(grid, top_pad + 2, left, '└');
        for c in (left + 1)..right {
            put(grid, top_pad + 2, c, '─');
        }
        put(grid, top_pad + 2, right, '┘');
        return;
    }

    // Divider between header and attribute table.
    put(grid, top_pad + 2, left, '├');
    for c in (left + 1)..right {
        put(grid, top_pad + 2, c, '─');
    }
    put(grid, top_pad + 2, right, '┤');

    // Attribute rows. Left-align each column; pad the row to the
    // box's interior width so the right border lines up.
    let cols = attr_columns(entity);
    for (i, attr) in entity.attributes.iter().enumerate() {
        let row = top_pad + HEADER_ROWS + i;
        put(grid, row, left, '│');
        let mut col = left + 1 + NAME_PAD;
        put_str(grid, row, col, &pad_right(&attr.type_name, cols.type_w));
        col += cols.type_w + 1;
        put_str(grid, row, col, &pad_right(&attr.name, cols.name_w));
        col += cols.name_w + 1;
        let keys_str = format_keys(&attr.keys);
        put_str(grid, row, col, &pad_right(&keys_str, cols.keys_w));
        put(grid, row, right, '│');
    }

    // Bottom border.
    let bottom = top_pad + HEADER_ROWS + entity.attributes.len();
    put(grid, bottom, left, '└');
    for c in (left + 1)..right {
        put(grid, bottom, c, '─');
    }
    put(grid, bottom, right, '┘');
}

/// Pad `s` with trailing spaces to exactly `width` cells of display
/// width. Used for attribute-table column alignment.
fn pad_right(s: &str, width: usize) -> String {
    let current = s.width();
    if current >= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + (width - current));
    out.push_str(s);
    for _ in current..width {
        out.push(' ');
    }
    out
}

/// Compact keys-column rendering: `PK`, `FK`, `UK`, comma-separated
/// when multiple. Matches Mermaid's web notation.
fn format_keys(keys: &[AttributeKey]) -> String {
    keys.iter()
        .map(|k| match k {
            AttributeKey::PrimaryKey => "PK",
            AttributeKey::ForeignKey => "FK",
            AttributeKey::UniqueKey => "UK",
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Draw a relationship as a horizontal line connecting the two
/// entity boxes. The line sits on the entity-name row (always row
/// `top_pad + 1`), passes THROUGH the side borders via `┤` / `├`
/// tee glyphs, and carries cardinality markers immediately past
/// each border. The optional label sits centred on the gap row
/// just above the boxes (`top_pad - 1`, i.e. row 0 — only present
/// when `top_pad >= 1`).
#[allow(clippy::too_many_arguments)]
fn draw_relationship_line(
    grid: &mut [Vec<char>],
    top_pad: usize,
    from_left: usize,
    from_width: usize,
    to_left: usize,
    to_width: usize,
    rel: &Relationship,
) {
    // Line lives on the entity-name row, always present in every
    // entity (even bare ones — it's the centre row of the 3-row
    // header). Both boxes share this row.
    let line_row = top_pad + 1;

    // Source/target border columns. `going_right` captures whether
    // the source entity sits to the left of the target — picks
    // which side border to merge into.
    let from_right_border = from_left + from_width - 1;
    let to_left_border = to_left;
    let from_left_border = from_left;
    let to_right_border = to_left + to_width - 1;
    let going_right = from_right_border < to_left_border;

    // `lo` and `hi` are the cells of the gap immediately past each
    // border (where cardinality glyphs land). The line fills the
    // cells between them.
    let (left_border, right_border, source_at_left, line_lo, line_hi) = if going_right {
        let lo = from_right_border + 1;
        let hi = to_left_border.saturating_sub(1);
        (from_right_border, to_left_border, true, lo, hi)
    } else {
        let lo = to_right_border + 1;
        let hi = from_left_border.saturating_sub(1);
        (to_right_border, from_left_border, false, lo, hi)
    };

    if line_hi <= line_lo {
        return; // entities touch or overlap; can't draw a line
    }

    let line_glyph = if rel.line_style.is_dashed() {
        '┄'
    } else {
        '─'
    };

    // 1. Merge the side borders with tee glyphs so the line meets
    //    each box visually. Don't merge dashed lines — `┤`/`├` are
    //    solid-only in box-drawing block; for dashed relationships
    //    we keep `│` and let the line just touch it (still reads as
    //    a connection because the cardinality glyph is adjacent).
    if !rel.line_style.is_dashed() {
        put(grid, line_row, left_border, '┤');
        put(grid, line_row, right_border, '├');
    }

    // 2. Fill the line across the gap.
    for c in line_lo..=line_hi {
        put(grid, line_row, c, line_glyph);
    }

    // 3. Cardinality glyphs at each end of the line. The glyph at
    //    `line_lo` always belongs to whichever endpoint sits on the
    //    LEFT side of the gap; `line_hi` to the right. So when the
    //    source is on the left (`source_at_left`), source-cardinality
    //    goes at `line_lo`; otherwise it goes at `line_hi`.
    let (lo_card, hi_card) = if source_at_left {
        (rel.from_cardinality, rel.to_cardinality)
    } else {
        (rel.to_cardinality, rel.from_cardinality)
    };
    put(grid, line_row, line_lo, cardinality_glyph(lo_card));
    put(grid, line_row, line_hi, cardinality_glyph(hi_card));

    // 4. Label centred above the line in the gap area, on the
    //    top-pad row. Only writes if the caller reserved a row for
    //    labels (top_pad >= 1) and the label fits in the gap.
    if top_pad == 0 {
        return;
    }
    if let Some(label) = &rel.label
        && !label.is_empty()
    {
        let label_w = label.width();
        let gap_w = line_hi - line_lo + 1;
        let label_row = top_pad - 1;
        if gap_w >= label_w {
            let offset = (gap_w - label_w) / 2;
            put_str(grid, label_row, line_lo + offset, label);
        } else {
            // Label wider than the gap — fall back to placing it at
            // the source end. Better to clip than to overwrite a
            // box border.
            put_str(grid, label_row, line_lo, label);
        }
    }
}

/// Single-character glyph at a relationship endpoint, conveying
/// cardinality. Chosen to read unambiguously in any monospace font:
///
/// - `1` — exactly one (mandatory single)
/// - `?` — zero or one (optional single)
/// - `+` — one or many (mandatory plural, `1+`)
/// - `*` — zero or many (optional plural, regex-style)
///
/// Mermaid's web renderer uses crow's-foot notation here; in
/// monospace text a single-character marker reads much more cleanly
/// than any multi-cell approximation of the branching lines.
fn cardinality_glyph(c: Cardinality) -> char {
    match c {
        Cardinality::ExactlyOne => '1',
        Cardinality::ZeroOrOne => '?',
        Cardinality::OneOrMany => '+',
        Cardinality::ZeroOrMany => '*',
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
    for (c, ch) in (col..).zip(s.chars()) {
        put(grid, row, c, ch);
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
        // Cardinality glyphs — `1` on the source (||, ExactlyOne)
        // and `*` on the target (o{, ZeroOrMany) — must appear on
        // the arrow row at each endpoint.
        assert!(out.contains('1'));
        assert!(out.contains('*'));
        assert!(out.contains("places"));
    }

    #[test]
    fn renders_isolated_entity_with_attributes() {
        let chart = parse("erDiagram\nCUSTOMER {\n  string name\n  string email PK\n}").unwrap();
        let out = render(&chart, None);
        assert!(out.contains("CUSTOMER"));
        // Attribute rows should render inside the box.
        assert!(out.contains("string"));
        assert!(out.contains("email"));
        assert!(out.contains("PK"));
    }

    #[test]
    fn renders_dashed_line_for_non_identifying() {
        let chart = parse("erDiagram\nA ||..o{ B").unwrap();
        let out = render(&chart, None);
        assert!(out.contains('┄'), "expected dashed line in:\n{out}");
    }

    #[test]
    fn cardinality_glyph_table_is_distinct() {
        // All four glyphs must be visually distinguishable so
        // readers don't have to cross-reference a legend.
        let glyphs = [
            cardinality_glyph(Cardinality::ExactlyOne),
            cardinality_glyph(Cardinality::ZeroOrOne),
            cardinality_glyph(Cardinality::OneOrMany),
            cardinality_glyph(Cardinality::ZeroOrMany),
        ];
        let unique: std::collections::HashSet<_> = glyphs.iter().collect();
        assert_eq!(unique.len(), 4, "cardinality glyphs must be unique");
    }

    #[test]
    fn format_keys_handles_zero_one_and_multiple() {
        assert_eq!(format_keys(&[]), "");
        assert_eq!(format_keys(&[AttributeKey::PrimaryKey]), "PK");
        assert_eq!(
            format_keys(&[AttributeKey::ForeignKey, AttributeKey::UniqueKey]),
            "FK,UK"
        );
    }
}
