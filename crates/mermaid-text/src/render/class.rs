//! Renderer for [`ClassDiagram`] (UML class diagrams).
//!
//! # Layout strategy
//!
//! 1. Synthesise an internal [`Graph`] with one [`Node`] per class. The node's
//!    `label` is crafted so that [`Node::label_width`] and
//!    [`Node::label_line_count`] return the desired box dimensions — the
//!    layered layout allocates correctly-sized grid cells.
//! 2. Call [`layout::layered::layout`] (top-to-bottom direction) to compute
//!    `(col, row)` positions.
//! 3. Allocate a `Vec<Vec<char>>` canvas and paint class boxes via
//!    `paint_class_box`.
//! 4. Connect each relation pair with a Manhattan L-route drawn directly into
//!    the `char` canvas.
//! 5. Paint class-diagram–specific endpoint glyphs (triangles, diamonds, arrows).
//!
//! # Default direction
//!
//! The default direction is **TB** (top-to-bottom), matching the most common
//! reading order for UML class hierarchies. This deviates from Mermaid's web
//! renderer default of BT (bottom-to-top) — the TB choice aligns better with
//! the terminal reading convention where parents appear above children.
//!
//! # Endpoint glyphs (class-diagram specific)
//!
//! These are **not** added to [`crate::types::EdgeEndpoint`] because they are
//! class-diagram–specific and would pollute the shared flowchart type. Instead
//! they are painted directly from polyline direction analysis.
//!
//! | Relationship            | Unicode glyph | ASCII fallback |
//! |-------------------------|---------------|----------------|
//! | Inheritance / Realization | `△`         | `^`            |
//! | Composition             | `◆`           | `#`            |
//! | Aggregation             | `◇`           | `*`            |
//! | Directed association    | `▸`/`▾`/`◂`/`▴` (existing arrow glyphs) | |
//! | Plain / Dependency      | no tip glyph  |                |

use unicode_width::UnicodeWidthStr;

use crate::class::{Class, ClassDiagram, Member, RelKind, Relation};
use crate::layout::layered::{LayoutConfig, LayoutResult, layout as layered_layout};
use crate::render::box_table::{NAME_PAD, grid_to_string, put, put_str};
use crate::types::{Direction, Edge, EdgeEndpoint, EdgeStyle, Graph, Node, NodeShape};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Interior horizontal padding on each side of box content.
const INTERIOR_PAD: usize = NAME_PAD;

/// Minimum gap (terminal cells) between adjacent class boxes (added on top of
/// the layered layout's own gap so boxes never visually merge).
const BOX_GAP: usize = 2;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render a `classDiagram` as a Unicode box-drawing string.
///
/// The optional `max_width` is accepted for API parity with other renderers
/// but v1 does not apply width-driven compaction — the diagram renders at its
/// natural size.
///
/// # Returns
///
/// A multi-line `String` containing the diagram rendered with Unicode
/// box-drawing characters. Returns an empty string when `chart.classes` is
/// empty.
pub fn render(chart: &ClassDiagram, _max_width: Option<usize>) -> String {
    if chart.classes.is_empty() {
        return String::new();
    }

    // 1. Compute per-class box geometry.
    let boxes: Vec<BoxGeometry> = chart.classes.iter().map(compute_box_geometry).collect();

    // 2. Synthesise the internal Graph so we can reuse the layered layout.
    let graph = synthesise_graph(chart, &boxes);

    // 3. Run layout (TB direction).
    let config = LayoutConfig {
        layer_gap: 4,
        node_gap: 2,
        ..LayoutConfig::default()
    };
    let LayoutResult { positions } = layered_layout(&graph, &config);

    if positions.is_empty() {
        return String::new();
    }

    // 4. Canvas dimensions.
    let canvas_cols = positions
        .iter()
        .map(|(name, &(col, _row))| {
            let w = boxes
                .iter()
                .find(|b| b.class_name == *name)
                .map_or(0, |b| b.box_width);
            col + w
        })
        .max()
        .unwrap_or(1)
        + BOX_GAP;

    let canvas_rows = positions
        .iter()
        .map(|(name, &(_col, row))| {
            let h = boxes
                .iter()
                .find(|b| b.class_name == *name)
                .map_or(0, |b| b.box_height);
            row + h
        })
        .max()
        .unwrap_or(1)
        + BOX_GAP;

    let mut grid: Vec<Vec<char>> = vec![vec![' '; canvas_cols]; canvas_rows];

    // 5. Paint class boxes.
    for (class, geo) in chart.classes.iter().zip(boxes.iter()) {
        let Some(&(col, row)) = positions.get(&class.name) else {
            continue;
        };
        paint_class_box(&mut grid, row, col, class, geo);
    }

    // 6. Paint relations as Manhattan polylines.
    for rel in &chart.relations {
        let Some(&(from_col, from_row)) = positions.get(&rel.from) else {
            continue;
        };
        let Some(&(to_col, to_row)) = positions.get(&rel.to) else {
            continue;
        };
        let from_geo = boxes.iter().find(|b| b.class_name == rel.from);
        let to_geo = boxes.iter().find(|b| b.class_name == rel.to);
        let (Some(fg), Some(tg)) = (from_geo, to_geo) else {
            continue;
        };
        paint_relation(&mut grid, from_col, from_row, fg, to_col, to_row, tg, rel);
    }

    grid_to_string(&grid)
}

// ---------------------------------------------------------------------------
// Box geometry
// ---------------------------------------------------------------------------

/// Precomputed dimensions for a class box.
pub(crate) struct BoxGeometry {
    /// The class name (key for position lookup).
    pub class_name: String,
    /// Total box width in terminal cells (including borders).
    pub box_width: usize,
    /// Total box height in rows (including borders).
    pub box_height: usize,
}

/// Compute the box geometry for a single class.
fn compute_box_geometry(class: &Class) -> BoxGeometry {
    let has_stereotype = class.stereotype.is_some();

    // Header content width: name or stereotype label, whichever is wider.
    let name_w = class.name.width();
    let stereo_w = class
        .stereotype
        .as_ref()
        .map(|s| format!("<<{}>>", s.label()).width())
        .unwrap_or(0);
    let header_content_w = name_w.max(stereo_w);
    // box_width = border (1) + pad + content + pad + border (1).
    let header_box_w = header_content_w + 2 * INTERIOR_PAD + 2;

    // Member rows width: visibility (1) + space + content.
    let member_content_w = class
        .members
        .iter()
        .map(member_display_width)
        .max()
        .unwrap_or(0);
    let member_box_w = if member_content_w > 0 {
        member_content_w + 2 * INTERIOR_PAD + 2
    } else {
        0
    };

    let box_width = header_box_w.max(member_box_w).max(6);

    // Height: top_border(1) + stereotype?(1) + name(1) + divider?(1) + members + bottom(1)
    let body_rows = if !class.members.is_empty() {
        1 + class.members.len() // divider + member rows
    } else {
        0
    };
    let box_height = 1 // top border
        + if has_stereotype { 1 } else { 0 }
        + 1 // name row
        + body_rows
        + 1; // bottom border

    BoxGeometry {
        class_name: class.name.clone(),
        box_width,
        box_height,
    }
}

/// Display width of a formatted member line (without the leading INTERIOR_PAD).
fn member_display_width(m: &Member) -> usize {
    format_member(m).width()
}

// ---------------------------------------------------------------------------
// Graph synthesis
// ---------------------------------------------------------------------------

/// Build an internal [`Graph`] from the class diagram to reuse the layered
/// layout. Each class becomes a [`Node`] with a crafted label whose width and
/// line count match the box dimensions (minus borders).
fn synthesise_graph(chart: &ClassDiagram, boxes: &[BoxGeometry]) -> Graph {
    let mut graph = Graph::new(Direction::TopToBottom);

    for (class, geo) in chart.classes.iter().zip(boxes.iter()) {
        // Interior dimensions (excluding borders).
        let interior_w = geo.box_width.saturating_sub(2);
        let interior_h = geo.box_height.saturating_sub(2);
        // Craft a label that drives label_width() = interior_w and
        // label_line_count() = interior_h. The label text is never rendered —
        // paint_class_box draws the actual box.
        let filler_line = " ".repeat(interior_w.max(1));
        let label = std::iter::repeat_n(filler_line.as_str(), interior_h.max(1))
            .collect::<Vec<_>>()
            .join("\n");

        graph
            .nodes
            .push(Node::new(&class.name, label, NodeShape::Rectangle));
    }

    for rel in &chart.relations {
        graph.edges.push(Edge {
            from: rel.from.clone(),
            to: rel.to.clone(),
            label: None,
            style: if rel.kind.is_dashed() {
                EdgeStyle::Dotted
            } else {
                EdgeStyle::Solid
            },
            end: EdgeEndpoint::None,
            start: EdgeEndpoint::None,
        });
    }

    graph
}

// ---------------------------------------------------------------------------
// Box painting
// ---------------------------------------------------------------------------

/// Paint a class box onto the character grid at position `(col, row)`.
fn paint_class_box(
    grid: &mut [Vec<char>],
    row: usize,
    col: usize,
    class: &Class,
    geo: &BoxGeometry,
) {
    let left = col;
    let right = col + geo.box_width - 1;
    let interior_w = geo.box_width - 2;

    // Top border.
    put(grid, row, left, '┌');
    for c in (left + 1)..right {
        put(grid, row, c, '─');
    }
    put(grid, row, right, '┐');

    let mut cur_row = row + 1;

    // Optional stereotype row: `<<name>>` centred.
    if let Some(stereo) = &class.stereotype {
        let label = format!("<<{}>>", stereo.label());
        let lw = label.width();
        let offset = (interior_w.saturating_sub(lw)) / 2;
        put(grid, cur_row, left, '│');
        put_str(grid, cur_row, left + 1 + offset, &label);
        put(grid, cur_row, right, '│');
        cur_row += 1;
    }

    // Name row — centred.
    {
        let name_w = class.name.width();
        let offset = (interior_w.saturating_sub(name_w)) / 2;
        put(grid, cur_row, left, '│');
        put_str(grid, cur_row, left + 1 + offset, &class.name);
        put(grid, cur_row, right, '│');
        cur_row += 1;
    }

    if class.members.is_empty() {
        // No body — close with bottom border.
        put(grid, cur_row, left, '└');
        for c in (left + 1)..right {
            put(grid, cur_row, c, '─');
        }
        put(grid, cur_row, right, '┘');
        return;
    }

    // Divider between header and body.
    put(grid, cur_row, left, '├');
    for c in (left + 1)..right {
        put(grid, cur_row, c, '─');
    }
    put(grid, cur_row, right, '┤');
    cur_row += 1;

    // Member rows — left-aligned with INTERIOR_PAD indent.
    for member in &class.members {
        let text = format_member(member);
        put(grid, cur_row, left, '│');
        put_str(grid, cur_row, left + 1 + INTERIOR_PAD, &text);
        put(grid, cur_row, right, '│');
        cur_row += 1;
    }

    // Bottom border.
    put(grid, cur_row, left, '└');
    for c in (left + 1)..right {
        put(grid, cur_row, c, '─');
    }
    put(grid, cur_row, right, '┘');
}

/// Format a member for display inside a class box.
fn format_member(m: &Member) -> String {
    match m {
        Member::Attribute(a) => {
            let vis = a
                .visibility
                .map(|v| v.as_char().to_string())
                .unwrap_or_else(|| " ".to_string());
            let suffix = if a.is_static { "$" } else { "" };
            if a.type_name.is_empty() {
                format!("{vis}{}{suffix}", a.name)
            } else {
                format!("{vis}{} {}{suffix}", a.name, a.type_name)
            }
        }
        Member::Method(mt) => {
            let vis = mt
                .visibility
                .map(|v| v.as_char().to_string())
                .unwrap_or_else(|| " ".to_string());
            let ret = mt
                .return_type
                .as_ref()
                .map(|r| format!(" {r}"))
                .unwrap_or_default();
            let static_s = if mt.is_static { "$" } else { "" };
            let abs_s = if mt.is_abstract { "*" } else { "" };
            format!("{vis}{}({}){ret}{static_s}{abs_s}", mt.name, mt.params)
        }
    }
}

// ---------------------------------------------------------------------------
// Relation painting
// ---------------------------------------------------------------------------

/// Paint a relation between two boxes as a Manhattan L-route directly on the
/// character grid. The route goes from the nearest border midpoint of the
/// source box to the nearest border midpoint of the target box.
///
/// Routing strategy:
/// 1. Identify the best attach face on each box (based on which side is closer
///    to the opposite box centre).
/// 2. Draw a two-segment (L-shaped) or straight path between the attach points.
/// 3. Paint endpoint glyphs for the relation kind.
/// 4. Paint the optional label at the midpoint.
#[allow(clippy::too_many_arguments)]
fn paint_relation(
    grid: &mut [Vec<char>],
    from_col: usize,
    from_row: usize,
    from_geo: &BoxGeometry,
    to_col: usize,
    to_row: usize,
    to_geo: &BoxGeometry,
    rel: &Relation,
) {
    // Midpoints of each box.
    let from_cx = from_col + from_geo.box_width / 2;
    let from_cy = from_row + from_geo.box_height / 2;
    let to_cx = to_col + to_geo.box_width / 2;
    let to_cy = to_row + to_geo.box_height / 2;

    // Attach faces: prefer the face that is closest to the other box.
    let dy = to_cy as isize - from_cy as isize;
    let dx = to_cx as isize - from_cx as isize;

    // Determine dominant axis for the first hop.
    let line_ch = if rel.kind.is_dashed() { '┄' } else { '─' };
    let vert_ch = if rel.kind.is_dashed() { '┆' } else { '│' };

    if dy.abs() >= dx.abs() {
        // Vertical dominant — prefer top/bottom attach.
        let (from_ac, from_ar, to_ac, to_ar) = if dy > 0 {
            // From is above To — exit from bottom, enter To's top.
            let far = from_row + from_geo.box_height - 1;
            (from_cx, far, to_cx, to_row)
        } else {
            // From is below To — exit from top, enter To's bottom.
            (from_cx, from_row, to_cx, to_row + to_geo.box_height - 1)
        };
        draw_manhattan(grid, from_ac, from_ar, to_ac, to_ar, line_ch, vert_ch);
        paint_endpoints(grid, from_ac, from_ar, to_ac, to_ar, rel, false);
    } else {
        // Horizontal dominant — prefer left/right attach.
        let (from_ac, from_ar, to_ac, to_ar) = if dx > 0 {
            // From is left of To — exit from right, enter To's left.
            let far = from_col + from_geo.box_width - 1;
            (far, from_cy, to_col, to_cy)
        } else {
            // From is right of To — exit from left, enter To's right.
            (from_col, from_cy, to_col + to_geo.box_width - 1, to_cy)
        };
        draw_manhattan(grid, from_ac, from_ar, to_ac, to_ar, line_ch, vert_ch);
        paint_endpoints(grid, from_ac, from_ar, to_ac, to_ar, rel, true);
    }

    // Paint optional label above the midpoint of the line.
    if let Some(label) = &rel.label
        && !label.is_empty()
    {
        // Label goes one row above the line's midpoint (in TB layout).
        let mid_col = (from_cx + to_cx) / 2;
        let mid_row = (from_row + from_geo.box_height / 2 + to_row + to_geo.box_height / 2) / 2;
        let label_row = mid_row.saturating_sub(1);
        put_str(grid, label_row, mid_col, label);
    }
}

/// Draw a Manhattan (L-shaped or straight) path from `(c0, r0)` to `(c1, r1)`.
///
/// Uses vertical-then-horizontal routing: travel vertically from `r0` to `r1`,
/// then horizontally to `c1`. This produces clean paths in a TB layout because
/// the first hop exits the source box vertically.
fn draw_manhattan(
    grid: &mut [Vec<char>],
    c0: usize,
    r0: usize,
    c1: usize,
    r1: usize,
    h_ch: char,
    v_ch: char,
) {
    // Vertical segment: from r0 to r1 along column c0.
    if r0 != r1 {
        let (r_lo, r_hi) = if r0 < r1 { (r0, r1) } else { (r1, r0) };
        for r in r_lo..=r_hi {
            let cur = grid
                .get(r)
                .and_then(|row| row.get(c0))
                .copied()
                .unwrap_or(' ');
            // Don't overwrite a horizontal line with a vertical glyph if already
            // there — produce a cross junction instead.
            let ch = junction(cur, v_ch, false);
            put(grid, r, c0, ch);
        }
    }

    // Horizontal segment: from c0 to c1 along row r1.
    if c0 != c1 {
        let (c_lo, c_hi) = if c0 < c1 { (c0, c1) } else { (c1, c0) };
        for c in c_lo..=c_hi {
            let cur = grid
                .get(r1)
                .and_then(|row| row.get(c))
                .copied()
                .unwrap_or(' ');
            let ch = junction(cur, h_ch, true);
            put(grid, r1, c, ch);
        }
    }
}

/// Compute the glyph for a cell where a new line crosses an existing one.
///
/// When `is_horizontal` is `true`, we're painting a horizontal segment;
/// otherwise a vertical one. If the existing cell already has a perpendicular
/// line, we produce `┼`; if same-axis, we keep the new glyph; if empty, we
/// paint the new glyph.
fn junction(existing: char, new_ch: char, is_horizontal: bool) -> char {
    let h_chars = ['─', '┄', '━', '┼', '├', '┤', '┬', '┴'];
    let v_chars = ['│', '┆', '┃', '┼', '├', '┤', '┬', '┴'];
    let existing_is_h = h_chars.contains(&existing);
    let existing_is_v = v_chars.contains(&existing);

    if is_horizontal && existing_is_v {
        // Horizontal line crossing a vertical — produce cross.
        '┼'
    } else if !is_horizontal && existing_is_h {
        // Vertical line crossing a horizontal — produce cross.
        '┼'
    } else {
        new_ch
    }
}

/// Paint endpoint glyphs at the source (`from_ac, from_ar`) and target
/// (`to_ac, to_ar`) attach points.
///
/// `horizontal` indicates the direction of the first segment:
/// - `true`  → the line first goes horizontally (LR layout dominant)
/// - `false` → the line first goes vertically (TB layout dominant)
fn paint_endpoints(
    grid: &mut [Vec<char>],
    from_ac: usize,
    from_ar: usize,
    to_ac: usize,
    to_ar: usize,
    rel: &Relation,
    horizontal: bool,
) {
    // Determine the travel direction at each attach point.
    let from_dir = if horizontal {
        if to_ac > from_ac {
            Dir::Right
        } else {
            Dir::Left
        }
    } else if to_ar > from_ar {
        Dir::Down
    } else {
        Dir::Up
    };
    let to_dir = from_dir.reverse();

    match rel.kind {
        RelKind::Inheritance | RelKind::Realization => {
            // Hollow triangle `△` at the FROM end (the parent/interface).
            // `A <|-- B` means from=A (parent), to=B (child).
            put(grid, from_ar, from_ac, '△');
        }
        RelKind::Composition => {
            // Filled diamond `◆` at the FROM end (the owner/whole).
            put(grid, from_ar, from_ac, '◆');
        }
        RelKind::Aggregation => {
            // Hollow diamond `◇` at the FROM end.
            put(grid, from_ar, from_ac, '◇');
        }
        RelKind::AssociationDirected => {
            // Arrow at the TO end.
            put(grid, to_ar, to_ac, dir_arrow(to_dir));
        }
        RelKind::Dependency => {
            // Dashed arrow at the TO end.
            put(grid, to_ar, to_ac, dir_arrow(to_dir));
        }
        RelKind::AssociationPlain => {
            // No endpoint glyphs.
        }
    }
}

/// Cardinal direction for glyph selection.
#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Reverse (opposite) direction.
    fn reverse(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Arrow glyph pointing in the direction of travel.
fn dir_arrow(dir: Dir) -> char {
    match dir {
        Dir::Up => '▴',
        Dir::Down => '▾',
        Dir::Left => '◂',
        Dir::Right => '▸',
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::class::parse;

    #[test]
    fn render_empty_diagram_returns_empty_string() {
        let diag = ClassDiagram::default();
        assert_eq!(render(&diag, None), "");
    }

    #[test]
    fn render_single_bare_class_contains_name() {
        let diag = parse("classDiagram\nclass Animal").unwrap();
        let out = render(&diag, None);
        assert!(out.contains("Animal"), "missing name in:\n{out}");
        assert!(out.contains('┌'), "missing top border in:\n{out}");
        assert!(out.contains('└'), "missing bottom border in:\n{out}");
    }

    #[test]
    fn render_class_with_members_contains_members() {
        let src = "classDiagram\nclass Animal {\n    +String name\n    +speak() void\n}";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        assert!(out.contains("Animal"), "missing class name in:\n{out}");
        assert!(out.contains("name"), "missing attribute in:\n{out}");
        assert!(out.contains("speak"), "missing method in:\n{out}");
    }

    #[test]
    fn render_stereotype_appears_in_box() {
        let src = "classDiagram\nclass IShape {\n    <<interface>>\n    +draw()\n}";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        assert!(
            out.contains("<<interface>>"),
            "missing stereotype in:\n{out}"
        );
    }

    #[test]
    fn render_two_classes_with_relation_produces_both_names() {
        let src = "classDiagram\nAnimal <|-- Dog";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        assert!(out.contains("Animal"), "missing Animal in:\n{out}");
        assert!(out.contains("Dog"), "missing Dog in:\n{out}");
    }

    #[test]
    fn render_no_panic_on_all_relation_kinds() {
        let kinds = [
            "A <|-- B", "A --|> B", "A *-- B", "A o-- B", "A --> B", "A -- B", "A <|.. B",
            "A ..|> B", "A ..> B", "A <.. B",
        ];
        for src in kinds {
            let full = format!("classDiagram\n{src}");
            let diag = parse(&full).unwrap();
            let out = render(&diag, None);
            assert!(!out.is_empty(), "empty render for {src:?}");
        }
    }

    #[test]
    fn format_member_attribute_with_type() {
        let m = Member::Attribute(crate::class::Attribute {
            visibility: Some(crate::class::Visibility::Public),
            name: "count".to_string(),
            type_name: "int".to_string(),
            is_static: false,
        });
        assert_eq!(format_member(&m), "+count int");
    }

    #[test]
    fn format_member_method_with_params_and_return() {
        let m = Member::Method(crate::class::Method {
            visibility: Some(crate::class::Visibility::Public),
            name: "add".to_string(),
            params: "x: int".to_string(),
            return_type: Some("int".to_string()),
            is_static: false,
            is_abstract: false,
        });
        assert_eq!(format_member(&m), "+add(x: int) int");
    }

    #[test]
    fn format_member_static_adds_dollar() {
        let m = Member::Attribute(crate::class::Attribute {
            visibility: Some(crate::class::Visibility::Public),
            name: "INSTANCE".to_string(),
            type_name: "Singleton".to_string(),
            is_static: true,
        });
        let s = format_member(&m);
        assert!(s.ends_with('$'), "expected '$' suffix in {s:?}");
    }

    #[test]
    fn render_class_with_relation_and_label() {
        let src = "classDiagram\nAnimal <|-- Dog : inherits";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        assert!(out.contains("inherits"), "missing label in:\n{out}");
    }

    #[test]
    fn render_output_has_no_trailing_whitespace_on_any_line() {
        let src = "classDiagram\nclass Animal {\n    +String name\n}\nclass Dog\nAnimal <|-- Dog";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        for (i, line) in out.lines().enumerate() {
            assert_eq!(
                line,
                line.trim_end(),
                "trailing whitespace on line {i}: {line:?}"
            );
        }
    }
}
