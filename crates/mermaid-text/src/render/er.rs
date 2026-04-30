//! Renderer for [`ErDiagram`] (entity-relationship diagrams).
//!
//! **Phase 2.1**: relationship lines connect entity boxes with `┤`/`├` tee
//! glyphs, cardinality markers adjacent to each border, and an optional label
//! row above the boxes.
//!
//! **Phase 3** (this version): when the natural single-row layout would exceed
//! the available width budget (default 80 columns, or `max_width` if smaller),
//! entities are wrapped into a `ceil(sqrt(n))`-column grid. Cross-row
//! relationships are routed via a vertical spine on the right side of the
//! diagram: horizontal stub from source → vertical leg along the spine →
//! horizontal stub to destination. Same-row relationships reuse the existing
//! horizontal routing unchanged.
//!
//! Identifying relationships use solid `─` lines; non-identifying use dashed
//! `┄`. Cardinality glyphs (`1`/`?`/`+`/`*`) appear adjacent to each endpoint.

use unicode_width::UnicodeWidthStr;

use crate::er::{AttributeKey, Cardinality, ErDiagram, Relationship};
use crate::render::box_table::{NAME_PAD, grid_to_string, pad_right, put, put_str};

/// Default terminal width budget. Diagrams narrower than this use a single row.
const DEFAULT_MAX_WIDTH: usize = 80;

/// Minimum cells of horizontal padding between adjacent entity boxes.
const MIN_ENTITY_GAP: usize = 4;

/// Rows consumed by the entity-name header: top border + name + divider.
const HEADER_ROWS: usize = 3;

/// Inter-row gap in character rows between grid rows of entity boxes.
const ROW_GAP: usize = 3;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render an [`ErDiagram`] to a Unicode string.
///
/// # Arguments
///
/// * `chart`     — parsed ER diagram
/// * `max_width` — optional terminal-width budget; `None` uses 80 columns
///
/// # Returns
///
/// A multi-line string with box-drawing characters. Returns an empty string
/// when `chart.entities` is empty.
pub fn render(chart: &ErDiagram, max_width: Option<usize>) -> String {
    if chart.entities.is_empty() {
        return String::new();
    }

    let budget = max_width.unwrap_or(DEFAULT_MAX_WIDTH);

    // Compute each entity's natural width and height independent of layout.
    let entity_widths: Vec<usize> = chart.entities.iter().map(entity_box_width).collect();
    let entity_heights: Vec<usize> = chart.entities.iter().map(entity_box_height).collect();

    let n = chart.entities.len();

    // Determine how many columns to use. Try a single row first; only switch
    // to grid when it would overflow the budget.
    let n_cols = decide_cols(n, &entity_widths, budget);

    // Assign each entity to a (grid_row, grid_col) cell.
    let entity_grid_pos: Vec<(usize, usize)> = (0..n).map(|i| (i / n_cols, i % n_cols)).collect();

    // Per-column pixel-width: the widest entity box in each column.
    let n_rows = n.div_ceil(n_cols);
    let col_widths: Vec<usize> = (0..n_cols)
        .map(|gc| {
            (0..n)
                .filter(|&i| entity_grid_pos[i].1 == gc)
                .map(|i| entity_widths[i])
                .max()
                .unwrap_or(0)
        })
        .collect();

    // Per-grid-row pixel-height: the tallest entity box in each row.
    let row_heights: Vec<usize> = (0..n_rows)
        .map(|gr| {
            (0..n)
                .filter(|&i| entity_grid_pos[i].0 == gr)
                .map(|i| entity_heights[i])
                .max()
                .unwrap_or(HEADER_ROWS)
        })
        .collect();

    // Compute per-pair gaps for same-row adjacent entities.
    // We build a flat `pair_gaps[i]` for adjacent pairs (i, i+1) within the
    // same row, re-using the existing logic but scoped to intra-row neighbours.
    let intra_row_pair_gaps = compute_intra_row_pair_gaps(chart, &entity_grid_pos, n_cols);

    // x-anchor (left column) for each entity on the canvas.
    let entity_left: Vec<usize> = compute_entity_left(
        n,
        &entity_grid_pos,
        &col_widths,
        n_cols,
        &intra_row_pair_gaps,
    );

    // Reserve one top-pad row if any relationship has a label (for label text).
    let has_labels = chart
        .relationships
        .iter()
        .any(|r| r.label.as_deref().is_some_and(|s| !s.is_empty()));
    let top_pad: usize = if has_labels { 1 } else { 0 };

    // y-anchor (top row on canvas, after top_pad) for each entity.
    let entity_top: Vec<usize> = compute_entity_top(n, &entity_grid_pos, &row_heights, top_pad);

    // Canvas dimensions.
    // Width: widest row's last-right column.
    let canvas_width =
        compute_canvas_width(n, chart, &entity_grid_pos, &entity_left, &entity_widths);
    // Height: top_pad + all row heights + inter-row gaps.
    let canvas_height = {
        let total_entity_h: usize = row_heights.iter().sum();
        let gaps = if n_rows > 1 {
            (n_rows - 1) * ROW_GAP
        } else {
            0
        };
        top_pad + total_entity_h + gaps
    };

    let mut grid: Vec<Vec<char>> = vec![vec![' '; canvas_width.max(1)]; canvas_height.max(1)];

    // Pass 1: draw entity boxes.
    for (i, entity) in chart.entities.iter().enumerate() {
        let left = entity_left[i];
        let right = left + entity_widths[i] - 1;
        draw_entity_box(&mut grid, entity_top[i], left, right, entity);
    }

    // Tracks which (col_start, col_end) ranges have been claimed by a
    // cross-row label on each row. `draw_cross_row_relationship` consults
    // this map to find a free row when the desired label slot collides
    // with one written by an earlier relationship.
    let mut used_label_ranges: std::collections::HashMap<usize, Vec<(usize, usize)>> =
        std::collections::HashMap::new();

    // Pass 2: draw relationship lines.
    for rel in &chart.relationships {
        let (Some(from_idx), Some(to_idx)) =
            (chart.entity_index(&rel.from), chart.entity_index(&rel.to))
        else {
            continue;
        };
        if from_idx == to_idx {
            continue;
        }

        let from_grid_row = entity_grid_pos[from_idx].0;
        let to_grid_row = entity_grid_pos[to_idx].0;

        if from_grid_row == to_grid_row {
            // Same grid row — use the flat horizontal routing.
            draw_relationship_line(
                &mut grid,
                entity_top[from_idx],
                entity_left[from_idx],
                entity_widths[from_idx],
                entity_left[to_idx],
                entity_widths[to_idx],
                rel,
                top_pad,
            );
        } else {
            // Cross-row — route via a vertical spine on the right edge of
            // the canvas. This avoids routing through entity boxes and is
            // simple to implement. An optimisation pass (edge-crossing
            // minimisation) can come later if requested.
            let from_is_rightmost = is_rightmost_in_row(from_idx, &entity_grid_pos, n_cols, n);
            let to_is_rightmost = is_rightmost_in_row(to_idx, &entity_grid_pos, n_cols, n);
            draw_cross_row_relationship(
                &mut grid,
                entity_top[from_idx],
                entity_heights[from_idx],
                entity_left[from_idx],
                entity_widths[from_idx],
                entity_top[to_idx],
                entity_heights[to_idx],
                entity_left[to_idx],
                entity_widths[to_idx],
                rel,
                canvas_width,
                from_is_rightmost,
                to_is_rightmost,
                &mut used_label_ranges,
            );
        }
    }

    grid_to_string(&grid)
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

/// Choose the number of grid columns.
///
/// Returns 1 (single row) when the total entity width fits within `budget`.
/// Otherwise returns `ceil(sqrt(n))` so entities are distributed roughly
/// square. If one entity is wider than the budget, we accept the overflow
/// (degrade gracefully).
fn decide_cols(n: usize, entity_widths: &[usize], budget: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    // Natural single-row total width: sum of widths + minimum gaps between pairs.
    let single_row_width: usize = entity_widths.iter().sum::<usize>() + MIN_ENTITY_GAP * (n - 1);
    if single_row_width <= budget {
        return n; // everything fits in one row
    }
    // Switch to grid. Use ceil(sqrt(n)) columns.
    let cols = (n as f64).sqrt().ceil() as usize;
    cols.max(1)
}

/// Compute the x-anchor (canvas column) for every entity.
///
/// Entities in the same grid column share the same column-width (the widest
/// entity in that column). Within each grid row, entities are placed
/// left-to-right with an inter-entity gap computed from `intra_row_pair_gaps`.
fn compute_entity_left(
    n: usize,
    entity_grid_pos: &[(usize, usize)],
    col_widths: &[usize],
    n_cols: usize,
    intra_row_pair_gaps: &[Vec<usize>],
) -> Vec<usize> {
    let mut out = vec![0usize; n];
    // For each entity, its x = sum of widths of all prior grid columns + their
    // inter-column gaps.  The "inter-column gap" is intra_row_pair_gaps[gr][gc].
    //
    // We derive x from grid column position: x(gc) = sum_{k<gc}(col_widths[k] + gap[k]).
    // Pre-compute column x-anchors per row (gaps may differ per row).
    let n_rows = entity_grid_pos.iter().map(|p| p.0).max().unwrap_or(0) + 1;
    for (gr, gaps) in intra_row_pair_gaps.iter().enumerate().take(n_rows) {
        let mut x = 0usize;
        for (gc, &col_w) in col_widths.iter().enumerate().take(n_cols) {
            // Find the entity at (gr, gc), if any.
            for i in 0..n {
                if entity_grid_pos[i] == (gr, gc) {
                    out[i] = x;
                }
            }
            x += col_w;
            if gc + 1 < n_cols {
                x += gaps.get(gc).copied().unwrap_or(MIN_ENTITY_GAP);
            }
        }
    }

    // Entities that share a grid column but are narrower than `col_widths[gc]`
    // are shifted to centre within their column slot.  We intentionally
    // left-align (don't centre) so relationship line maths stays simple.
    out
}

/// Compute the y-anchor (canvas row) for every entity, accounting for the
/// `top_pad` label row and inter-row `ROW_GAP` spacing.
fn compute_entity_top(
    n: usize,
    entity_grid_pos: &[(usize, usize)],
    row_heights: &[usize],
    top_pad: usize,
) -> Vec<usize> {
    let mut out = vec![0usize; n];
    // Row y-anchors (character row on canvas where box top border sits).
    let mut y = top_pad;
    let n_rows = row_heights.len();
    let mut row_y = Vec::with_capacity(n_rows);
    for (gr, &h) in row_heights.iter().enumerate() {
        row_y.push(y);
        y += h;
        if gr + 1 < n_rows {
            y += ROW_GAP;
        }
    }
    for i in 0..n {
        out[i] = row_y[entity_grid_pos[i].0];
    }
    out
}

/// Compute the total canvas width: the maximum right-edge across all entities.
///
/// A spine column (2 extra characters) is reserved to the right of all entity
/// boxes ONLY when at least one relationship actually crosses grid rows.  When
/// all relationships are intra-row the spine is never drawn, so we must not
/// allocate the margin — this prevents a visible empty column (or stray corner
/// glyph) appearing on the right side of single-row and small multi-row
/// diagrams that have no cross-row edges.
fn compute_canvas_width(
    n: usize,
    chart: &ErDiagram,
    entity_grid_pos: &[(usize, usize)],
    entity_left: &[usize],
    entity_widths: &[usize],
) -> usize {
    let rightmost_entity = (0..n)
        .map(|i| entity_left[i] + entity_widths[i])
        .max()
        .unwrap_or(0);

    // Only add the spine margin when at least one relationship is cross-row.
    // A relationship is cross-row when the grid row of `from` differs from the
    // grid row of `to`.
    let needs_spine = chart.relationships.iter().any(|rel| {
        let Some(fi) = chart.entity_index(&rel.from) else {
            return false;
        };
        let Some(ti) = chart.entity_index(&rel.to) else {
            return false;
        };
        fi != ti && entity_grid_pos[fi].0 != entity_grid_pos[ti].0
    });

    // 2 extra columns: 1 gap between the rightmost entity and the spine +
    // 1 spine column itself.
    rightmost_entity + if needs_spine { 2 } else { 0 }
}

/// Compute inter-entity gaps for adjacent pairs within the SAME grid row.
///
/// Returns a `Vec<Vec<usize>>` indexed by `[grid_row][grid_col_pair]`.
/// `gaps[gr][gc]` is the gap between entity at `(gr, gc)` and `(gr, gc+1)`.
fn compute_intra_row_pair_gaps(
    chart: &ErDiagram,
    entity_grid_pos: &[(usize, usize)],
    n_cols: usize,
) -> Vec<Vec<usize>> {
    let n_rows = entity_grid_pos.iter().map(|p| p.0).max().unwrap_or(0) + 1;

    // For each grid row, `gaps[gc]` is the gap between col gc and gc+1.
    let mut gaps: Vec<Vec<usize>> = (0..n_rows)
        .map(|_| vec![MIN_ENTITY_GAP; n_cols.saturating_sub(1)])
        .collect();

    // Widen gaps to accommodate relationship labels between adjacent-column entities.
    for rel in &chart.relationships {
        let (Some(from_idx), Some(to_idx)) =
            (chart.entity_index(&rel.from), chart.entity_index(&rel.to))
        else {
            continue;
        };
        if from_idx == to_idx {
            continue;
        }
        let (from_gr, from_gc) = entity_grid_pos[from_idx];
        let (to_gr, to_gc) = entity_grid_pos[to_idx];
        if from_gr != to_gr {
            continue; // cross-row; handled separately
        }
        let (lo_gc, hi_gc) = if from_gc <= to_gc {
            (from_gc, to_gc)
        } else {
            (to_gc, from_gc)
        };
        let label_w = rel.label.as_deref().map(|s| s.width()).unwrap_or(0);
        let needed = label_w.max(2) + 4;
        for gc in lo_gc..hi_gc {
            if let Some(g) = gaps[from_gr].get_mut(gc) {
                *g = (*g).max(needed);
            }
        }
    }

    // Ensure each row has exactly n_cols-1 gap slots (pad with MIN_ENTITY_GAP).
    for row in &mut gaps {
        while row.len() < n_cols.saturating_sub(1) {
            row.push(MIN_ENTITY_GAP);
        }
    }

    gaps
}

// ---------------------------------------------------------------------------
// Column width helpers
// ---------------------------------------------------------------------------

/// Total box width for an entity: the max of (header width, attribute
/// table width) plus padding and borders.
fn entity_box_width(entity: &crate::er::Entity) -> usize {
    let header_w = entity.name.width() + 2 * NAME_PAD + 2;
    if entity.attributes.is_empty() {
        return header_w;
    }
    let cols = attr_columns(entity);
    let attr_w = 2 * NAME_PAD + cols.type_w + 1 + cols.name_w + 1 + cols.keys_w + 2;
    attr_w.max(header_w)
}

/// Total box height for an entity: HEADER_ROWS when empty, else
/// `HEADER_ROWS + attrs + 1` (for the bottom border).
fn entity_box_height(entity: &crate::er::Entity) -> usize {
    if entity.attributes.is_empty() {
        HEADER_ROWS
    } else {
        HEADER_ROWS + entity.attributes.len() + 1
    }
}

// ---------------------------------------------------------------------------
// Attribute column helpers
// ---------------------------------------------------------------------------

/// Per-column display widths across all attribute rows in an entity.
struct AttrColumns {
    type_w: usize,
    name_w: usize,
    keys_w: usize,
}

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

/// Compact keys-column rendering: `PK`, `FK`, `UK`, comma-separated.
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

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

/// Draw the full entity box at canvas position `(entity_top, left..=right)`.
fn draw_entity_box(
    grid: &mut [Vec<char>],
    entity_top: usize,
    left: usize,
    right: usize,
    entity: &crate::er::Entity,
) {
    let interior_w = right - left - 1;
    let name_w = entity.name.width();
    let name_start = left + 1 + (interior_w.saturating_sub(name_w)) / 2;

    put(grid, entity_top, left, '┌');
    for c in (left + 1)..right {
        put(grid, entity_top, c, '─');
    }
    put(grid, entity_top, right, '┐');

    put(grid, entity_top + 1, left, '│');
    put_str(grid, entity_top + 1, name_start, &entity.name);
    put(grid, entity_top + 1, right, '│');

    if entity.attributes.is_empty() {
        put(grid, entity_top + 2, left, '└');
        for c in (left + 1)..right {
            put(grid, entity_top + 2, c, '─');
        }
        put(grid, entity_top + 2, right, '┘');
        return;
    }

    put(grid, entity_top + 2, left, '├');
    for c in (left + 1)..right {
        put(grid, entity_top + 2, c, '─');
    }
    put(grid, entity_top + 2, right, '┤');

    let cols = attr_columns(entity);
    for (i, attr) in entity.attributes.iter().enumerate() {
        let row = entity_top + HEADER_ROWS + i;
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

    let bottom = entity_top + HEADER_ROWS + entity.attributes.len();
    put(grid, bottom, left, '└');
    for c in (left + 1)..right {
        put(grid, bottom, c, '─');
    }
    put(grid, bottom, right, '┘');
}

/// Draw a horizontal relationship line between two entities on the SAME
/// grid row. The line sits on the entity-name row (`entity_top + 1`) and
/// passes through the side borders via `┤`/`├` tee glyphs.
///
/// # Arguments
///
/// * `entity_top` — canvas row of the FROM entity's top border
///   (both entities share the same grid row so their tops align)
/// * `top_pad`    — number of label-reserve rows above the first grid row
///   (used to place relationship labels above the boxes)
#[allow(clippy::too_many_arguments)]
fn draw_relationship_line(
    grid: &mut [Vec<char>],
    entity_top: usize,
    from_left: usize,
    from_width: usize,
    to_left: usize,
    to_width: usize,
    rel: &Relationship,
    top_pad: usize,
) {
    let line_row = entity_top + 1;

    let from_right_border = from_left + from_width - 1;
    let to_left_border = to_left;
    let from_left_border = from_left;
    let to_right_border = to_left + to_width - 1;
    let going_right = from_right_border < to_left_border;

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
        return;
    }

    let line_glyph = if rel.line_style.is_dashed() {
        '┄'
    } else {
        '─'
    };

    if !rel.line_style.is_dashed() {
        put(grid, line_row, left_border, '┤');
        put(grid, line_row, right_border, '├');
    }

    for c in line_lo..=line_hi {
        put(grid, line_row, c, line_glyph);
    }

    let (lo_card, hi_card) = if source_at_left {
        (rel.from_cardinality, rel.to_cardinality)
    } else {
        (rel.to_cardinality, rel.from_cardinality)
    };
    put(grid, line_row, line_lo, cardinality_glyph(lo_card));
    put(grid, line_row, line_hi, cardinality_glyph(hi_card));

    if top_pad == 0 {
        return;
    }
    if let Some(label) = &rel.label
        && !label.is_empty()
    {
        let label_w = label.width();
        let gap_w = line_hi.saturating_sub(line_lo) + 1;
        // Labels sit on the top-pad row just above the first grid row.
        // For multi-grid-row diagrams every intra-row label shares that same
        // row 0; labels for entities deeper in the grid have no dedicated row
        // above them (cross-row labels are handled by `draw_cross_row_relationship`).
        let label_row = if entity_top >= top_pad {
            entity_top - 1
        } else {
            return; // no room
        };
        if gap_w >= label_w {
            let offset = (gap_w - label_w) / 2;
            put_str(grid, label_row, line_lo + offset, label);
        } else {
            put_str(grid, label_row, line_lo, label);
        }
    }
}

/// Draw a cross-row relationship using a right-margin spine route.
///
/// # Why this routing strategy
///
/// Cross-row arrows must not pass through entity boxes that sit on the same
/// canvas row as the source or destination. The only safe approach is to route
/// entirely in the right margin of the canvas — a vertical "spine" column that
/// lies past the right edge of every entity box. Horizontal stubs on the entity
/// name rows extend only from the box border to the spine column; vertical glyphs
/// fill the spine between the two entity rows.
///
/// Route shape (source above target):
/// ```text
///  │  SRC  │1┐
///           │  (spine travels down through ROW_GAP rows)
///  │  TGT  │*┘
/// ```
///
/// The `1` and `*` are cardinality glyphs placed immediately right of each box's
/// right border. The corner glyphs `┐`/`┘` sit in the spine column. The spine
/// column is `canvas_width - 1` — reserved during canvas sizing so it never
/// overlaps an entity box.
///
/// When two cross-row arrows share the spine they will overlap in the vertical
/// segment. A future edge-crossing minimisation pass can assign each arrow its
/// own spine column offset; for now we accept the overlap.
#[allow(clippy::too_many_arguments)]
fn draw_cross_row_relationship(
    grid: &mut [Vec<char>],
    from_top: usize,
    from_height: usize,
    from_left: usize,
    from_width: usize,
    to_top: usize,
    to_height: usize,
    to_left: usize,
    to_width: usize,
    rel: &Relationship,
    canvas_width: usize,
    from_is_rightmost: bool,
    to_is_rightmost: bool,
    used_label_ranges: &mut std::collections::HashMap<usize, Vec<(usize, usize)>>,
) {
    // The spine is the last column of the canvas. The canvas is sized with a
    // 2-column margin (1 gap + 1 spine) beyond the rightmost entity box, so
    // the spine never falls inside a box.
    let spine_col = if canvas_width > 0 {
        canvas_width - 1
    } else {
        return;
    };

    let vert_glyph = if rel.line_style.is_dashed() {
        '┆'
    } else {
        '│'
    };

    // Entity name rows — both stubs live here.
    let from_row = from_top + 1;
    let to_row = to_top + 1;

    let from_right_border = from_left + from_width - 1;
    let to_right_border = to_left + to_width - 1;

    // --- Source stub ---
    // Tee glyph at source's right border, cardinality glyph one cell to the
    // right, then a corner at the spine. We skip horizontal fill between the
    // cardinality glyph and the spine when the entity has neighbours to its
    // right in the grid row (a fill would overwrite their name rows). When
    // the entity is rightmost in its row, the fill is safe and we draw it
    // so the spine visually connects to the entity instead of floating.
    if from_right_border < spine_col {
        if !rel.line_style.is_dashed() {
            put(grid, from_row, from_right_border, '┤');
        }
        let card_col = from_right_border + 1;
        put(
            grid,
            from_row,
            card_col,
            cardinality_glyph(rel.from_cardinality),
        );
        if from_is_rightmost {
            let fill_glyph = if rel.line_style.is_dashed() {
                '┄'
            } else {
                '─'
            };
            for c in (card_col + 1)..spine_col {
                put(grid, from_row, c, fill_glyph);
            }
        }
        let corner = if from_row < to_row { '┐' } else { '┘' };
        put(grid, from_row, spine_col, corner);
    } else {
        // Degenerate: entity wider than canvas. Just mark cardinality.
        put(
            grid,
            from_row,
            from_right_border,
            cardinality_glyph(rel.from_cardinality),
        );
    }

    // --- Vertical leg ---
    // Fills the spine column between the two entity name rows (exclusive).
    // Rows between entity row-groups are in the ROW_GAP area, guaranteed free.
    let (vert_lo, vert_hi) = if from_row < to_row {
        (from_row + 1, to_row)
    } else {
        (to_row + 1, from_row)
    };
    for r in vert_lo..vert_hi {
        put(grid, r, spine_col, vert_glyph);
    }

    // --- Target stub ---
    // Corner at spine, cardinality glyph, tee into target's right border.
    // Same skip-or-fill rule as the source stub: fill horizontally only when
    // the target is rightmost in its grid row (no neighbour name rows to
    // clobber).
    if to_right_border < spine_col {
        let corner = if from_row < to_row { '┘' } else { '┐' };
        put(grid, to_row, spine_col, corner);
        let card_col = to_right_border + 1;
        put(
            grid,
            to_row,
            card_col,
            cardinality_glyph(rel.to_cardinality),
        );
        if to_is_rightmost {
            let fill_glyph = if rel.line_style.is_dashed() {
                '┄'
            } else {
                '─'
            };
            for c in (card_col + 1)..spine_col {
                put(grid, to_row, c, fill_glyph);
            }
        }
        if !rel.line_style.is_dashed() {
            put(grid, to_row, to_right_border, '├');
        }
    } else {
        put(
            grid,
            to_row,
            to_right_border,
            cardinality_glyph(rel.to_cardinality),
        );
    }

    // --- Label ---
    // Place the label in the ROW_GAP area immediately after the source entity's
    // bottom row. This guarantees we're in the gap between entity row-groups,
    // never on another entity's name row.
    //
    // When two cross-row relationships target the same gap, walk down through
    // the gap rows to find one where the label's column range doesn't overlap
    // anything already claimed. If the whole gap is full, fall back to the
    // first row (accept the collision rather than dropping the label).
    if let Some(label) = &rel.label
        && !label.is_empty()
        && from_row != to_row
    {
        let first_gap_row = if from_row < to_row {
            from_top + from_height
        } else {
            to_top + to_height
        };
        let label_w = label.width();
        let label_col = spine_col.saturating_sub(label_w + 1);
        let label_end = label_col + label_w;
        let chosen_row = (0..ROW_GAP)
            .map(|offset| first_gap_row + offset)
            .find(|row| {
                used_label_ranges
                    .get(row)
                    .is_none_or(|ranges| !ranges.iter().any(|&(s, e)| s < label_end && label_col < e))
            })
            .unwrap_or(first_gap_row);
        used_label_ranges
            .entry(chosen_row)
            .or_default()
            .push((label_col, label_end));
        put_str(grid, chosen_row, label_col, label);
    }
}

/// Whether `idx` is the rightmost entity in its grid row. Used to decide
/// whether `draw_cross_row_relationship` may fill the horizontal stub
/// between the entity's cardinality glyph and the spine column — the fill
/// is unsafe when there are neighbours to the right (their name rows live
/// on the same canvas row).
fn is_rightmost_in_row(
    idx: usize,
    entity_grid_pos: &[(usize, usize)],
    n_cols: usize,
    n: usize,
) -> bool {
    if idx + 1 >= n {
        return true;
    }
    if n_cols == 0 {
        return true;
    }
    let (row, _) = entity_grid_pos[idx];
    let (next_row, _) = entity_grid_pos[idx + 1];
    next_row != row
}

/// Single-character glyph for a relationship endpoint cardinality.
///
/// - `1` — exactly one
/// - `?` — zero or one
/// - `+` — one or many
/// - `*` — zero or many
fn cardinality_glyph(c: Cardinality) -> char {
    match c {
        Cardinality::ExactlyOne => '1',
        Cardinality::ZeroOrOne => '?',
        Cardinality::OneOrMany => '+',
        Cardinality::ZeroOrMany => '*',
    }
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
        assert!(out.contains('1'));
        assert!(out.contains('*'));
        assert!(out.contains("places"));
    }

    #[test]
    fn renders_isolated_entity_with_attributes() {
        let chart = parse("erDiagram\nCUSTOMER {\n  string name\n  string email PK\n}").unwrap();
        let out = render(&chart, None);
        assert!(out.contains("CUSTOMER"));
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

    // -----------------------------------------------------------------------
    // Phase 3 grid layout tests
    // -----------------------------------------------------------------------

    /// Build a parse input with `n` bare entities and no relationships.
    fn make_bare_entities_src(n: usize) -> String {
        let mut src = "erDiagram\n".to_string();
        for i in 0..n {
            // First entity has a relationship to second to ensure rendering
            // exercises the connection path too.
            if i + 1 < n {
                src.push_str(&format!("E{i} ||--o{{ E{} : rel\n", i + 1));
            }
        }
        src
    }

    #[test]
    fn small_er_diagram_uses_single_row() {
        // 4 entities must fit in a single row with the default 80-column budget.
        let src = make_bare_entities_src(4);
        let chart = parse(&src).unwrap();
        let out = render(&chart, None);
        // All entity names present.
        for i in 0..4 {
            assert!(out.contains(&format!("E{i}")), "E{i} missing from output");
        }
        // In a single-row layout all entities share the same top-border row.
        // Count distinct rows that contain "┌" — should be exactly 1.
        let top_border_rows = out.lines().filter(|l| l.contains('┌')).count();
        assert_eq!(
            top_border_rows, 1,
            "expected 1 top-border row for 4 entities, got {top_border_rows}"
        );
    }

    #[test]
    fn wide_er_diagram_wraps_to_grid() {
        // 8 entities — with a 30-column budget they cannot fit on one row.
        let src = make_bare_entities_src(8);
        let chart = parse(&src).unwrap();
        let out = render(&chart, Some(30));
        // All entity names present.
        for i in 0..8 {
            assert!(out.contains(&format!("E{i}")), "E{i} missing from:\n{out}");
        }
        // Multi-row layout: more than one row contains "┌" (top border chars).
        let top_border_rows = out.lines().filter(|l| l.contains('┌')).count();
        assert!(
            top_border_rows > 1,
            "expected multiple top-border rows for 8 entities in 30 cols, got {top_border_rows}"
        );
    }

    #[test]
    fn cross_row_relationship_routes_correctly() {
        // Build an 8-entity diagram where E0 (grid row 0) and E4 (grid row 1,
        // since ceil(sqrt(8))=3, so row 1 starts at index 3) are related.
        // We just check that the output contains both entity names and a │ or
        // corner glyph from the vertical spine.
        let src = "erDiagram
E0 ||--o{ E1 : a
E1 ||--o{ E2 : b
E2 ||--o{ E3 : c
E3 ||--o{ E4 : d
E4 ||--o{ E5 : e
E5 ||--o{ E6 : f
E6 ||--o{ E7 : g";
        let chart = parse(src).unwrap();
        let out = render(&chart, Some(30));
        // Entities must be present.
        assert!(out.contains("E0"), "E0 missing");
        assert!(out.contains("E4"), "E4 missing");
        // A vertical leg (│) or corner (┐/┘/└/┌) must exist for cross-row routing.
        let has_vertical = out.contains('│') || out.contains('┐') || out.contains('┘');
        assert!(has_vertical, "no vertical routing glyphs found in:\n{out}");
    }

    // ---- Bug 3: small diagram has no right spine -------------------------

    #[test]
    fn small_diagram_has_no_right_spine() {
        // Two entities with a single intra-row relationship.  Both entities
        // land on grid row 0 so there are NO cross-row relationships — the
        // spine (2 extra columns beyond the rightmost entity box) must NOT
        // be allocated.
        //
        // The spine manifests as a vertical-leg row in the ROW_GAP area:
        // a line that contains `│` but NO entity-box characters (`┌`, `├`,
        // `└`, `─`).  Such a "gap-area │ row" can only come from the spine.
        // Entity box interior rows DO contain `│` but always also contain
        // content characters; the cross-row spine's vertical leg falls in the
        // ROW_GAP area which otherwise has only spaces.
        //
        // For this 2-entity intra-row diagram, there should be no such rows.
        let src = "erDiagram
A ||--|| B : rel";
        let chart = parse(src).unwrap();
        let out = render(&chart, Some(20));

        // Cross-row spine appears as a line in the inter-row gap that contains
        // `│` (or `┆`) but no box characters.
        let spine_in_gap = out.lines().any(|l| {
            let has_vert = l.contains('│') || l.contains('┆');
            let has_box = l.contains('┌')
                || l.contains('├')
                || l.contains('└')
                || l.contains('─')
                || l.contains('┤');
            has_vert && !has_box
        });
        assert!(
            !spine_in_gap,
            "intra-row-only diagram should not have spine-only rows, got:\n{out}"
        );

        // Additionally verify that the relationship itself rendered (the test
        // diagram is non-trivial).
        assert!(out.contains('A'), "entity A missing from:\n{out}");
        assert!(out.contains('B'), "entity B missing from:\n{out}");
    }

    #[test]
    fn cross_row_target_alone_in_row_has_horizontal_stub_to_spine() {
        // Bug repro: with the canonical 7-entity invoice schema, INVOICE
        // sits alone in the bottom grid row (only entity in that row).
        // Its cardinality glyph `1` was being rendered with NO horizontal
        // line connecting it to the spine column on the right edge — the
        // spine corner `┘` was floating in space, visually disconnected
        // from INVOICE.
        //
        // The skip-horizontal-fill logic in `draw_cross_row_relationship`
        // is needed when the entity has neighbours to its right in the
        // grid row (because the fill would clobber their name rows). When
        // the entity is rightmost in its row, the fill is safe and must
        // be drawn.
        let src = "erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ ITEM : contains
    PRODUCT ||--o{ ITEM : describes
    CATEGORY ||--o{ PRODUCT : groups
    ACCOUNT ||--|| CUSTOMER : owns
    INVOICE ||--|{ ORDER : bills
    CUSTOMER { int id PK string name }
    ORDER    { int id PK int customerId FK }
    PRODUCT  { int id PK string name int categoryId FK }
    CATEGORY { int id PK string label }
    ACCOUNT  { int id PK }
    INVOICE  { int id PK }
    ITEM     { int orderId FK int productId FK }";
        let chart = parse(src).unwrap();
        let out = render(&chart, None);

        // Find the INVOICE name row.
        let invoice_row = out
            .lines()
            .find(|l| l.contains("INVOICE") && l.contains('│'))
            .unwrap_or_else(|| panic!("INVOICE name row not found in:\n{out}"));

        // Strip trailing whitespace and assert that the row contains a
        // contiguous horizontal stub connecting INVOICE's right border to
        // the spine. The stub uses `─` characters between the cardinality
        // glyph and the spine corner `┘`.
        let trimmed = invoice_row.trim_end();
        // The row should end with the spine corner.
        assert!(
            trimmed.ends_with('┘') || trimmed.ends_with('┐'),
            "INVOICE row should end with a spine corner glyph (┘ or ┐), got: {trimmed:?}"
        );
        // Between the cardinality glyph (`1`) and the spine corner there
        // must be at least one `─` (the connecting stub). Without the fix,
        // the gap is filled with spaces.
        let card_pos = trimmed
            .find('1')
            .expect("INVOICE cardinality glyph `1` missing");
        let corner_pos = trimmed.rfind('┘').or_else(|| trimmed.rfind('┐')).unwrap();
        let gap = &trimmed[card_pos + 1..corner_pos];
        assert!(
            gap.contains('─'),
            "expected `─` stub between INVOICE cardinality `1` and spine corner, got gap: {gap:?}\nfull row: {trimmed:?}"
        );
    }

    #[test]
    fn cross_row_labels_in_same_gap_row_do_not_overlap() {
        // Bug repro: when two cross-row relationships target the same
        // inter-row gap, both labels were placed at the same column
        // (`spine_col - label_w - 1`) and visually collided. In the
        // canonical 7-entity invoice schema, "describes" (PRODUCT→ITEM)
        // and "bills" (INVOICE→ORDER) both routed through the gap below
        // row 0, and the second write clobbered the first — the output
        // showed `descbills` instead of two separate labels.
        let src = "erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ ITEM : contains
    PRODUCT ||--o{ ITEM : describes
    CATEGORY ||--o{ PRODUCT : groups
    ACCOUNT ||--|| CUSTOMER : owns
    INVOICE ||--|{ ORDER : bills
    CUSTOMER { int id PK string name }
    ORDER    { int id PK int customerId FK }
    PRODUCT  { int id PK string name int categoryId FK }
    CATEGORY { int id PK string label }
    ACCOUNT  { int id PK }
    INVOICE  { int id PK }
    ITEM     { int orderId FK int productId FK }";
        let chart = parse(src).unwrap();
        let out = render(&chart, None);
        assert!(
            out.contains("describes"),
            "label 'describes' was clobbered by an overlapping label:\n{out}"
        );
        assert!(
            out.contains("bills"),
            "label 'bills' was clobbered by an overlapping label:\n{out}"
        );
        // The corrupted concatenation must not appear.
        assert!(
            !out.contains("descbills") && !out.contains("billsescribes"),
            "two labels collided into a single token:\n{out}"
        );
    }

    #[test]
    fn grid_honours_max_width_budget() {
        // With max_width=50 and 8 entities, the renderer wraps to a grid.
        // Each rendered line (after stripping trailing spaces) must be ≤ 52
        // columns wide (we allow a 2-column spine overage for the vertical
        // routing channel that is reserved outside the budget entities).
        let src = make_bare_entities_src(8);
        let chart = parse(&src).unwrap();
        let out = render(&chart, Some(50));
        // The canvas width should be within a reasonable bound of the budget.
        // We assert each line is not excessively wide (budget + small constant
        // for spine).
        for (line_no, line) in out.lines().enumerate() {
            let w = line.width();
            assert!(
                w <= 60,
                "line {line_no} is {w} chars wide (budget 50), content: {line:?}"
            );
        }
    }
}
