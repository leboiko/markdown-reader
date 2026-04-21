//! Renderer for [`ErDiagram`] (entity-relationship diagrams).
//!
//! **Phase 2** (this version): multi-row entity boxes with attribute
//! tables (header + divider + per-attribute rows), plus cardinality
//! glyphs at each relationship endpoint (`1` exactly one, `?` zero
//! or one, `+` one or many, `*` zero or many). Identifying
//! relationships use solid `─` lines; non-identifying use dashed `┄`.
//!
//! Phase 3 (planned) replaces the single-row layout with a grid for
//! diagrams with more than ~4 entities.

use unicode_width::UnicodeWidthStr;

use crate::er::{AttributeKey, Cardinality, ErDiagram, Relationship};

/// Cells of horizontal padding between adjacent entity boxes in the
/// single-row layout. Big enough that relationship arrows have room
/// to label themselves between boxes; small enough that the row
/// fits comfortably in a typical terminal.
const ENTITY_GAP: usize = 4;

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
    let entity_widths: Vec<usize> =
        chart.entities.iter().map(entity_box_width).collect();
    let entity_heights: Vec<usize> =
        chart.entities.iter().map(entity_box_height).collect();
    let tallest = *entity_heights.iter().max().unwrap_or(&HEADER_ROWS);

    // Entities lay out left-to-right in source order with
    // `ENTITY_GAP` cells between each.
    let entity_left: Vec<usize> = {
        let mut out = Vec::with_capacity(chart.entities.len());
        let mut col = 0usize;
        for &w in &entity_widths {
            out.push(col);
            col += w + ENTITY_GAP;
        }
        out
    };

    let last_right = entity_left
        .last()
        .zip(entity_widths.last())
        .map(|(&left, &w)| left + w)
        .unwrap_or(0);

    // Canvas: tallest entity + 1 spacer + (2 rows per relationship).
    let relationship_rows = if chart.relationships.is_empty() {
        0
    } else {
        1 + chart.relationships.len() * 2
    };
    let height = tallest + relationship_rows;
    let width = last_right.max(1);

    let mut grid: Vec<Vec<char>> = vec![vec![' '; width]; height];

    // Pass 1: draw each entity box (header + attribute rows).
    for (i, entity) in chart.entities.iter().enumerate() {
        let left = entity_left[i];
        let right = left + entity_widths[i] - 1;
        draw_entity_box(&mut grid, left, right, entity);
    }

    // Pass 2: draw each relationship as a labelled arrow below the
    // entity row, with cardinality glyphs at both endpoints.
    for (rel_idx, rel) in chart.relationships.iter().enumerate() {
        let (Some(from_idx), Some(to_idx)) = (
            chart.entity_index(&rel.from),
            chart.entity_index(&rel.to),
        ) else {
            continue;
        };
        // Each relationship consumes two rows: label above, arrow
        // below, starting after the entity-row plus a 1-row spacer.
        let row_label = tallest + rel_idx * 2;
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
/// attributes) + one row per attribute + bottom border.
fn draw_entity_box(
    grid: &mut [Vec<char>],
    left: usize,
    right: usize,
    entity: &crate::er::Entity,
) {
    let interior_w = right - left - 1;
    let name_w = entity.name.width();
    let name_start = left + 1 + (interior_w.saturating_sub(name_w)) / 2;

    // Top border.
    put(grid, 0, left, '┌');
    for c in (left + 1)..right {
        put(grid, 0, c, '─');
    }
    put(grid, 0, right, '┐');

    // Name row — centre horizontally.
    put(grid, 1, left, '│');
    put_str(grid, 1, name_start, &entity.name);
    put(grid, 1, right, '│');

    if entity.attributes.is_empty() {
        // Bare entity — just a 3-row box (no divider, no rows).
        put(grid, 2, left, '└');
        for c in (left + 1)..right {
            put(grid, 2, c, '─');
        }
        put(grid, 2, right, '┘');
        return;
    }

    // Divider between header and attribute table.
    put(grid, 2, left, '├');
    for c in (left + 1)..right {
        put(grid, 2, c, '─');
    }
    put(grid, 2, right, '┤');

    // Attribute rows. Left-align each column; pad the row to the
    // box's interior width so the right border lines up.
    let cols = attr_columns(entity);
    for (i, attr) in entity.attributes.iter().enumerate() {
        let row = HEADER_ROWS + i;
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
    let bottom = HEADER_ROWS + entity.attributes.len();
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

/// Draw a single relationship line with cardinality glyphs at each
/// endpoint. The arrow row sits below the entity boxes; the label
/// (user text only — cardinalities are in the endpoint glyphs now)
/// sits on the row just above the arrow.
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
    let going_right = from_right_edge < to_left_edge;
    let (lo, hi) = if going_right {
        (from_right_edge, to_left_edge.saturating_sub(1))
    } else {
        (to_left + to_width, from_left.saturating_sub(1))
    };
    if hi <= lo + 2 {
        return; // not enough room for glyphs + line
    }

    let line_glyph = if rel.line_style.is_dashed() { '┄' } else { '─' };

    // Draw the connecting line across the gap.
    for c in lo..=hi {
        put(grid, row_arrow, c, line_glyph);
    }

    // Cardinality glyphs at each endpoint, one cell inside the line.
    // The glyph at `lo` marks the source cardinality; at `hi` marks
    // the target cardinality. Visible direction is captured by the
    // source-to-target reading order.
    let (source_card, target_card) = if going_right {
        (rel.from_cardinality, rel.to_cardinality)
    } else {
        (rel.to_cardinality, rel.from_cardinality)
    };
    put(grid, row_arrow, lo, cardinality_glyph(source_card));
    put(grid, row_arrow, hi, cardinality_glyph(target_card));

    // Label row: just the user-supplied label text (cardinalities
    // live on the arrow row now). Centre it over the line.
    if let Some(label) = &rel.label
        && !label.is_empty()
    {
        let label_w = label.width();
        let line_w = hi - lo;
        if line_w >= label_w {
            let label_col = lo + (line_w.saturating_sub(label_w)) / 2 + 1;
            put_str(grid, row_label, label_col, label);
        } else {
            // Line too short — just place at the source end so at
            // least the start of the label is visible.
            put_str(grid, row_label, lo, label);
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
        // Cardinality glyphs — `1` on the source (||, ExactlyOne)
        // and `*` on the target (o{, ZeroOrMany) — must appear on
        // the arrow row at each endpoint.
        assert!(out.contains('1'));
        assert!(out.contains('*'));
        assert!(out.contains("places"));
    }

    #[test]
    fn renders_isolated_entity_with_attributes() {
        let chart = parse(
            "erDiagram\nCUSTOMER {\n  string name\n  string email PK\n}",
        )
        .unwrap();
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
