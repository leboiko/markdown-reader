//! 2D character grid used for building the final text output.
//!
//! The grid stores one `char` per cell plus a parallel obstacle layer.
//! The obstacle layer is used by A\* edge routing to distinguish:
//!
//! - **Hard obstacles** — cells that belong to a node bounding box (walls
//!   and interior). Edges must not pass through these.
//! - **Soft obstacles** — cells already occupied by a previously-routed edge.
//!   Edges can cross these but at increased cost.
//!
//! All drawing operations write directly into the grid; the final string is
//! produced by converting the grid to a `String` via its [`std::fmt::Display`]
//! implementation.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use unicode_width::UnicodeWidthChar;

// Box-drawing character sets
/// Rectangle corners and sides. T-junctions and crosses are not listed here
/// because they are derived on demand by the direction-bit canvas via
/// [`DIR_TO_CHAR`].
mod rect {
    pub const TL: char = '┌';
    pub const TR: char = '┐';
    pub const BL: char = '└';
    pub const BR: char = '┘';
    pub const H: char = '─';
    pub const V: char = '│';
}

/// Rounded-corner box characters.
mod rounded {
    pub const TL: char = '╭';
    pub const TR: char = '╮';
    pub const BL: char = '╰';
    pub const BR: char = '╯';
}

/// Arrow tip characters.
pub mod arrow {
    pub const RIGHT: char = '▸';
    pub const DOWN: char = '▾';
    pub const LEFT: char = '◂';
    pub const UP: char = '▴';
}

/// Endpoint glyph characters for non-arrow edge terminations.
pub mod endpoint {
    /// Circle endpoint (`--o`).
    pub const CIRCLE: char = '○';
    /// Cross endpoint (`--x`).
    pub const CROSS: char = '×';
}

/// Dotted box-drawing characters (┆ for vertical, ┄ for horizontal).
///
/// Unicode's dotted box-drawing characters lack proper junction glyphs, so
/// dotted lines revert to solid junction characters where they meet other
/// edges. This is a documented compromise — see `render/unicode.rs` for the
/// explanation comment at the call site.
mod dotted {
    pub const H: char = '┄';
    pub const V: char = '┆';
}

/// Lookup table for thick line junctions: same 4-bit direction mask as
/// `DIR_TO_CHAR` but using thick Unicode glyphs.
const THICK_DIR_TO_CHAR: [char; 16] = [
    ' ', // 0000
    '┃', // 0001 UP
    '┃', // 0010 DOWN
    '┃', // 0011 UP+DOWN
    '━', // 0100 LEFT
    '┛', // 0101 UP+LEFT
    '┓', // 0110 DOWN+LEFT
    '┫', // 0111 UP+DOWN+LEFT
    '━', // 1000 RIGHT
    '┗', // 1001 UP+RIGHT
    '┏', // 1010 DOWN+RIGHT
    '┣', // 1011 UP+DOWN+RIGHT
    '━', // 1100 LEFT+RIGHT
    '┻', // 1101 UP+LEFT+RIGHT
    '┳', // 1110 DOWN+LEFT+RIGHT
    '╋', // 1111 cross
];

// ---------------------------------------------------------------------------
// Direction-bit canvas
// ---------------------------------------------------------------------------
//
// Each cell carries a 4-bit direction mask describing the line segments that
// exit the cell toward its neighbors. Writing a line segment OR-merges the
// appropriate bits into the cell, and the resulting bitmask is used to look
// up the correct box-drawing glyph. This produces correct T-junctions
// (`├ ┤ ┬ ┴`) and crosses (`┼`) for free whenever edges meet — the logic that
// used to live in `merge_h_line`/`merge_v_line`/`merge_corner_*` collapses
// into a single table lookup.

const DIR_UP: u8 = 0b0001;
const DIR_DOWN: u8 = 0b0010;
const DIR_LEFT: u8 = 0b0100;
const DIR_RIGHT: u8 = 0b1000;

/// Lookup table mapping a 4-bit direction mask (UP=1, DOWN=2, LEFT=4, RIGHT=8)
/// to the single box-drawing glyph that represents it.
///
/// Single-direction stubs (`╵╷╴╶`) would render as half-length line fragments
/// in most terminal fonts, so we use the full `│` / `─` instead — matching
/// termaid's chosen behavior for "edge segment that leaves a cell but didn't
/// enter from the expected opposite side".
const DIR_TO_CHAR: [char; 16] = [
    ' ', // 0000 — empty
    '│', // 0001 — UP only
    '│', // 0010 — DOWN only
    '│', // 0011 — UP+DOWN (plain vertical)
    '─', // 0100 — LEFT only
    '┘', // 0101 — UP+LEFT
    '┐', // 0110 — DOWN+LEFT
    '┤', // 0111 — UP+DOWN+LEFT
    '─', // 1000 — RIGHT only
    '└', // 1001 — UP+RIGHT
    '┌', // 1010 — DOWN+RIGHT
    '├', // 1011 — UP+DOWN+RIGHT
    '─', // 1100 — LEFT+RIGHT (plain horizontal)
    '┴', // 1101 — UP+LEFT+RIGHT
    '┬', // 1110 — DOWN+LEFT+RIGHT
    '┼', // 1111 — cross
];

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Obstacle classification
// ---------------------------------------------------------------------------

/// Cell-level obstacle classification for A\* routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Obstacle {
    /// Free cell — no extra routing cost.
    Free,
    /// Cell belongs to a node bounding box. Edges must not enter.
    NodeBox,
    /// Cell already has a routed edge. Crossings are allowed at extra cost.
    EdgeOccupied,
}

// ---------------------------------------------------------------------------
// A* state
// ---------------------------------------------------------------------------

/// A single entry in the A\* open-set priority queue.
///
/// We use a min-heap via [`BinaryHeap`], so we invert the comparison to turn
/// it into a min-heap (smallest `f_cost` first).
#[derive(Debug, Clone, Copy)]
struct AstarNode {
    /// `f = g + h` (total estimated cost through this node).
    f_cost: f32,
    /// Steps taken to reach this cell from the start.
    g_cost: f32,
    col: usize,
    row: usize,
    /// Direction we arrived from (encoded as 0=R,1=D,2=L,3=U, `u8::MAX`=start).
    dir: u8,
}

impl PartialEq for AstarNode {
    fn eq(&self, other: &Self) -> bool {
        self.f_cost == other.f_cost
    }
}

impl Eq for AstarNode {}

impl Ord for AstarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order so BinaryHeap is a min-heap.
        other
            .f_cost
            .partial_cmp(&self.f_cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for AstarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Edge line style
// ---------------------------------------------------------------------------

/// Line style to apply when overwriting a routed path.
///
/// Passed to [`Grid::overdraw_path_style`] after a path has been drawn with
/// solid glyphs by [`Grid::route_edge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeLineStyle {
    /// Leave the path as drawn (solid box-drawing chars from the dir-bit canvas).
    Solid,
    /// Replace horizontal cells with `┄` and vertical cells with `┆`.
    ///
    /// Junctions with other edges are left as solid characters because Unicode
    /// lacks dotted junction glyphs — this is the documented trade-off.
    Dotted,
    /// Replace path cells using thick box-drawing glyphs (`━`, `┃`, `╋`, etc.),
    /// recomputed from the existing direction bitmask.
    Thick,
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

/// A mutable 2D grid of characters, used as a canvas for rendering.
///
/// The grid uses `(col, row)` addressing with origin at top-left `(0, 0)`.
/// Writes outside the grid bounds are silently discarded.
#[derive(Debug, Clone)]
pub struct Grid {
    /// Row-major storage: `cells[row][col]`
    cells: Vec<Vec<char>>,
    /// Parallel obstacle layer: `obstacles[row][col]`
    obstacles: Vec<Vec<Obstacle>>,
    /// Parallel direction-bit layer used by [`Grid::add_dirs`] for junction
    /// merging. Each cell holds the OR of the `DIR_*` bits for every line
    /// segment that has been drawn into it.
    directions: Vec<Vec<u8>>,
    /// Cell-protection flags. Writes via [`Grid::add_dirs`] skip protected
    /// cells so that rounded corners, arrow tips, and node labels survive
    /// any subsequent edge routing that happens to cross them.
    protected: Vec<Vec<bool>>,
    /// Total columns.
    width: usize,
    /// Total rows.
    height: usize,
}

impl Grid {
    /// Construct a new grid filled with spaces.
    ///
    /// # Arguments
    ///
    /// * `width`  — number of columns
    /// * `height` — number of rows
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            cells: vec![vec![' '; width]; height],
            obstacles: vec![vec![Obstacle::Free; width]; height],
            directions: vec![vec![0u8; width]; height],
            protected: vec![vec![false; width]; height],
            width,
            height,
        }
    }

    /// OR the given direction bits into the cell at `(col, row)` and update
    /// the cell's glyph from the direction-to-char lookup table.
    ///
    /// Protected cells (rounded corners, arrow tips, labels) are left alone —
    /// their glyph is preserved and the direction bits are not recorded.
    /// Out-of-bounds writes are silently ignored.
    fn add_dirs(&mut self, col: usize, row: usize, bits: u8) {
        if row >= self.height || col >= self.width {
            return;
        }
        if self.protected[row][col] {
            return;
        }
        self.directions[row][col] |= bits;
        self.cells[row][col] = DIR_TO_CHAR[self.directions[row][col] as usize];
    }

    /// Mark a cell as protected — subsequent [`Grid::add_dirs`] calls will
    /// not touch it. Used for rounded corners, arrow tips, and label text
    /// that must survive edge routing.
    fn protect(&mut self, col: usize, row: usize) {
        if row < self.height && col < self.width {
            self.protected[row][col] = true;
        }
    }

    /// Mark all cells of a node bounding box as hard obstacles.
    ///
    /// This must be called for every node *before* routing any edges so that
    /// A\* routing can avoid node boxes.
    ///
    /// # Arguments
    ///
    /// * `col`, `row` — top-left corner of the node box
    /// * `w`, `h`     — bounding-box dimensions (including border cells)
    pub fn mark_node_box(&mut self, col: usize, row: usize, w: usize, h: usize) {
        for dy in 0..h {
            for dx in 0..w {
                let r = row + dy;
                let c = col + dx;
                if r < self.height && c < self.width {
                    self.obstacles[r][c] = Obstacle::NodeBox;
                }
            }
        }
    }

    /// Mark a single cell as a hard obstacle (equivalent to a node-box cell).
    ///
    /// Used by the subgraph renderer to mark border cells so A\* routing
    /// avoids routing edges through the subgraph border lines.
    pub fn mark_obstacle(&mut self, col: usize, row: usize) {
        if row < self.height && col < self.width {
            self.obstacles[row][col] = Obstacle::NodeBox;
        }
    }

    /// Expose the internal `protect` method as a public API.
    ///
    /// Protected cells are skipped by the direction-bit canvas writer so that
    /// subgraph border characters and labels survive subsequent edge routing.
    pub fn protect_cell(&mut self, col: usize, row: usize) {
        self.protect(col, row);
    }

    /// Remove the protection flag from a cell so that subsequent writes
    /// (including the direction-bit canvas writer) can modify it again.
    ///
    /// Used after [`Grid::route_edge`] places a tip glyph that we want to
    /// replace (e.g. converting an arrow tip to a circle endpoint or removing
    /// it for plain no-arrow lines).
    pub fn unprotect_cell(&mut self, col: usize, row: usize) {
        if row < self.height && col < self.width {
            self.protected[row][col] = false;
        }
    }

    /// Recompute the glyph for cell `(col, row)` from its direction-bit mask.
    ///
    /// Call this after [`Grid::unprotect_cell`] to let the direction-bit canvas
    /// produce the correct box-drawing character for a cell whose protection
    /// was previously holding a different glyph (e.g. an arrow tip that should
    /// now be a path character because the edge has no endpoint marker).
    pub fn recompute_cell_glyph(&mut self, col: usize, row: usize) {
        if row < self.height && col < self.width {
            let bits = self.directions[row][col];
            self.cells[row][col] = DIR_TO_CHAR[bits as usize];
        }
    }

    /// Write `ch` at position `(col, row)`.
    ///
    /// Out-of-bounds writes are silently ignored.
    pub fn set(&mut self, col: usize, row: usize, ch: char) {
        if row < self.height && col < self.width {
            self.cells[row][col] = ch;
        }
    }

    /// Read the character at `(col, row)`, returning `' '` for out-of-bounds.
    pub fn get(&self, col: usize, row: usize) -> char {
        if row < self.height && col < self.width {
            self.cells[row][col]
        } else {
            ' '
        }
    }

    /// Convert the grid to a `String`, stripping trailing spaces from each row.
    ///
    /// This is a convenience wrapper around the [`std::fmt::Display`] impl.
    pub fn render(&self) -> String {
        self.to_string()
    }

    // -----------------------------------------------------------------------
    // Box drawing
    // -----------------------------------------------------------------------

    /// Draw a rectangle box with square corners at `(col, row)` with the given
    /// `width` and `height` (in characters, including the border).
    ///
    /// Minimum usable size is 2×2 (all corners, no interior).
    pub fn draw_box(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        self.set(col, row, rect::TL);
        self.set(col + w - 1, row, rect::TR);
        self.set(col, row + h - 1, rect::BL);
        self.set(col + w - 1, row + h - 1, rect::BR);

        for x in (col + 1)..(col + w - 1) {
            self.set(x, row, rect::H);
            self.set(x, row + h - 1, rect::H);
        }
        for y in (row + 1)..(row + h - 1) {
            self.set(col, y, rect::V);
            self.set(col + w - 1, y, rect::V);
        }
    }

    /// Draw a rounded-corner box at `(col, row)`.
    pub fn draw_rounded_box(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        self.set(col, row, rounded::TL);
        self.set(col + w - 1, row, rounded::TR);
        self.set(col, row + h - 1, rounded::BL);
        self.set(col + w - 1, row + h - 1, rounded::BR);

        for x in (col + 1)..(col + w - 1) {
            self.set(x, row, rect::H);
            self.set(x, row + h - 1, rect::H);
        }
        for y in (row + 1)..(row + h - 1) {
            self.set(col, y, rect::V);
            self.set(col + w - 1, y, rect::V);
        }
    }

    /// Draw a diamond-style node box: a standard rectangle with `◇` markers
    /// overwriting the horizontal centre of the top and bottom edges.
    ///
    /// This is the termaid convention — readable at any terminal width and
    /// unambiguous even with proportional fonts.
    ///
    /// `w` and `h` are the total bounding-box dimensions.
    pub fn draw_diamond(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        // Draw the standard rectangle first.
        self.draw_box(col, row, w, h);

        // Overwrite the horizontal centre of the top and bottom edges with ◇.
        // Using `col + w / 2` gives a deterministic centre regardless of
        // whether `w` is odd or even.
        let cx = col + w / 2;
        self.set(cx, row, '◇');
        self.set(cx, row + h - 1, '◇');
    }

    /// Draw a stadium (capsule/pill) node: rounded box with `(` / `)` markers
    /// on the vertical midpoint of the left and right edges.
    ///
    /// Mermaid syntax: `([label])`
    pub fn draw_stadium(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 4 || h < 2 {
            return;
        }
        self.draw_rounded_box(col, row, w, h);
        // Place `(` and `)` markers one cell inside each rounded side at the
        // vertical midpoint. This distinguishes stadium from plain rounded.
        let mid_row = row + h / 2;
        self.set(col + 1, mid_row, '(');
        self.set(col + w - 2, mid_row, ')');
        self.protect(col + 1, mid_row);
        self.protect(col + w - 2, mid_row);
    }

    /// Draw a subroutine node: rectangle with an extra inner vertical bar (`│`)
    /// one cell inside each left and right border.
    ///
    /// Mermaid syntax: `[[label]]`
    pub fn draw_subroutine(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 4 || h < 2 {
            return;
        }
        self.draw_box(col, row, w, h);
        // Place inner vertical bars for all interior rows.
        for y in (row + 1)..(row + h - 1) {
            self.set(col + 1, y, rect::V);
            self.set(col + w - 2, y, rect::V);
        }
    }

    /// Draw a cylinder (database) node: a box where the top edge shows a
    /// top-arc row (`╭─╮` style) plus a second interior arc row (`╰─╯`) to
    /// suggest depth, and the bottom edge mirrors this.
    ///
    /// Layout (5 rows):
    /// ```text
    /// ╭────╮   ← top arc
    /// ╰────╯   ← inner arc (depth indicator)
    ///  text
    /// ╭────╮   ← inner arc at bottom
    /// ╰────╯   ← bottom arc
    /// ```
    ///
    /// Mermaid syntax: `[(label)]`
    /// Draw a cylinder (database) node.
    ///
    /// Mermaid syntax: `[(label)]`. Rendered as a rounded rectangle with a
    /// horizontal "lid line" one row below the top border, distinguishing it
    /// from a plain rounded rectangle while keeping the silhouette
    /// continuous (unlike a double-arc design which visually disconnects in
    /// monospace fonts).
    ///
    /// Minimum height is 4 rows: top border + lid + text + bottom border.
    /// For multi-line labels, `h` grows by the number of extra label lines.
    pub fn draw_cylinder(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 4 {
            return;
        }
        // Top rounded border.
        self.set(col, row, rounded::TL);
        self.set(col + w - 1, row, rounded::TR);
        for x in (col + 1)..(col + w - 1) {
            self.set(x, row, rect::H);
        }
        // Lid indicator: T-junctions into the side walls with a horizontal
        // line across. Visually signals "there's a cap on top", and the
        // `├`/`┤` characters keep the side walls continuous with what sits
        // above and below them.
        self.set(col, row + 1, '├');
        self.set(col + w - 1, row + 1, '┤');
        for x in (col + 1)..(col + w - 1) {
            self.set(x, row + 1, rect::H);
        }
        // Straight side walls through every interior row up to the bottom border.
        for y in (row + 2)..(row + h - 1) {
            self.set(col, y, rect::V);
            self.set(col + w - 1, y, rect::V);
        }
        // Bottom rounded border.
        self.set(col, row + h - 1, rounded::BL);
        self.set(col + w - 1, row + h - 1, rounded::BR);
        for x in (col + 1)..(col + w - 1) {
            self.set(x, row + h - 1, rect::H);
        }
    }

    /// Draw a hexagon node: rectangle with `<` / `>` markers at the vertical
    /// midpoint of the left and right edges (similar to diamond's `◇` markers
    /// but on the sides rather than top/bottom).
    ///
    /// Mermaid syntax: `{{label}}`
    pub fn draw_hexagon(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 4 || h < 2 {
            return;
        }
        self.draw_box(col, row, w, h);
        // Overwrite left/right midpoints with `<` / `>` markers.
        let mid_row = row + h / 2;
        self.set(col, mid_row, '<');
        self.set(col + w - 1, mid_row, '>');
        self.protect(col, mid_row);
        self.protect(col + w - 1, mid_row);
    }

    /// Draw an asymmetric (flag) node: rectangle with a `⟩` marker at the
    /// vertical midpoint of the right border.
    ///
    /// Mermaid syntax: `>label]`
    pub fn draw_asymmetric(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        self.draw_box(col, row, w, h);
        // Replace the vertical midpoint of the right border with `⟩`.
        let mid_row = row + h / 2;
        self.set(col + w - 1, mid_row, '⟩');
        self.protect(col + w - 1, mid_row);
    }

    /// Draw a parallelogram node: rectangle with `/` markers overwriting the
    /// top-left and bottom-right corners (lean-right style).
    ///
    /// Mermaid syntax: `[/label/]`
    pub fn draw_parallelogram(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        self.draw_box(col, row, w, h);
        // Slant markers at the corners.
        self.set(col, row, '/');
        self.set(col + w - 1, row + h - 1, '/');
        self.protect(col, row);
        self.protect(col + w - 1, row + h - 1);
    }

    /// Draw a trapezoid node: rectangle with `/` at top-left and `\` at
    /// top-right, indicating a wider top, narrower bottom.
    ///
    /// Mermaid syntax: `[/label\]`
    pub fn draw_trapezoid(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 2 || h < 2 {
            return;
        }
        self.draw_box(col, row, w, h);
        // Slant markers at top corners only.
        self.set(col, row, '/');
        self.set(col + w - 1, row, '\\');
        self.protect(col, row);
        self.protect(col + w - 1, row);
    }

    /// Draw a double-circle node: two concentric rounded boxes, with the inner
    /// one drawn 1 cell inside the outer on all sides.
    ///
    /// Minimum useful size is 5 wide × 5 tall to leave a visible inner ring.
    ///
    /// Mermaid syntax: `(((label)))`
    pub fn draw_double_circle(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 5 || h < 5 {
            return;
        }
        // Outer rounded box.
        self.draw_rounded_box(col, row, w, h);
        // Inner rounded box, 1 cell inside on all sides.
        self.draw_rounded_box(col + 1, row + 1, w - 2, h - 2);
    }

    // -----------------------------------------------------------------------
    // Text writing
    // -----------------------------------------------------------------------

    /// Write `text` starting at `(col, row)`.
    ///
    /// Each character advances the column by its display width (via
    /// `unicode-width`), so multi-byte characters are handled correctly.
    pub fn write_text(&mut self, col: usize, row: usize, text: &str) {
        let mut x = col;
        for ch in text.chars() {
            if x >= self.width {
                break;
            }
            self.set(x, row, ch);
            // Advance by Unicode display width (most chars = 1, CJK = 2)
            x += UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }

    /// Write `text` starting at `(col, row)` and protect every cell written
    /// so that subsequent direction-bit canvas writes (from edge routing) cannot
    /// overwrite the label characters.
    ///
    /// Use this for edge labels that must survive later routing passes.
    pub fn write_text_protected(&mut self, col: usize, row: usize, text: &str) {
        let mut x = col;
        for ch in text.chars() {
            if x >= self.width {
                break;
            }
            self.set(x, row, ch);
            self.protect(x, row);
            x += UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }

    // -----------------------------------------------------------------------
    // Arrow / path drawing
    // -----------------------------------------------------------------------

    /// Draw a horizontal line with an arrow tip at the right end.
    ///
    /// Draws `─` from `(col1, row)` to `(col2-1, row)` then `▸` at `col2`.
    /// If `col1 >= col2` nothing is drawn.
    pub fn draw_h_arrow(&mut self, col1: usize, row: usize, col2: usize) {
        if col1 >= col2 {
            return;
        }
        for x in col1..col2 {
            self.add_dirs(x, row, DIR_LEFT | DIR_RIGHT);
        }
        self.set(col2, row, arrow::RIGHT);
        self.protect(col2, row);
    }

    /// Draw a vertical line with an arrow tip at the bottom.
    ///
    /// Draws `│` from `(col, row1)` to `(col, row2-1)` then `▾` at `row2`.
    /// If `row1 >= row2` nothing is drawn.
    pub fn draw_v_arrow(&mut self, col: usize, row1: usize, row2: usize) {
        if row1 >= row2 {
            return;
        }
        for y in row1..row2 {
            self.add_dirs(col, y, DIR_UP | DIR_DOWN);
        }
        self.set(col, row2, arrow::DOWN);
        self.protect(col, row2);
    }

    /// Draw a right-angle path from `(col1, row1)` to `(col2, row2)`.
    ///
    /// For horizontal-primary flow (LR/RL): horizontal segment first, then
    /// vertical. The corner is drawn as a junction character. An arrow tip
    /// is placed at the destination.
    ///
    /// For vertical-primary flow (TD/BT): vertical segment first, then
    /// horizontal.
    ///
    /// `horizontal_first` controls which axis is traversed first.
    pub fn draw_manhattan(
        &mut self,
        col1: usize,
        row1: usize,
        col2: usize,
        row2: usize,
        horizontal_first: bool,
        arrow_direction: char,
    ) {
        if col1 == col2 && row1 == row2 {
            return;
        }

        if horizontal_first {
            // Horizontal segment from (col1, row1) up to (but not including)
            // the corner at (col2, row1).
            if col1 != col2 {
                let (lo, hi) = order(col1, col2);
                for x in lo..hi {
                    self.add_dirs(x, row1, DIR_LEFT | DIR_RIGHT);
                }
            }

            if row1 == row2 {
                // Pure horizontal — arrow tip at the destination end.
                self.set(col2, row2, arrow_direction);
                self.protect(col2, row2);
            } else {
                // Corner at (col2, row1): incoming-horizontal side + outgoing-vertical side.
                let h_in = if col2 > col1 { DIR_LEFT } else { DIR_RIGHT };
                let v_out = if row2 > row1 { DIR_DOWN } else { DIR_UP };
                self.add_dirs(col2, row1, h_in | v_out);

                // Vertical segment between the corner and the tip (exclusive of both).
                let (vlo, vhi) = order(row1, row2);
                // `order` always gives (min, max). The corner sits at the min or max
                // depending on direction; the line cells are strictly between them.
                for y in (vlo + 1)..vhi {
                    self.add_dirs(col2, y, DIR_UP | DIR_DOWN);
                }

                self.set(col2, row2, arrow_direction);
                self.protect(col2, row2);
            }
        } else {
            // Vertical segment up to (but not including) the corner at (col1, row2).
            if row1 != row2 {
                let (lo, hi) = order(row1, row2);
                for y in lo..hi {
                    self.add_dirs(col1, y, DIR_UP | DIR_DOWN);
                }
            }

            if col1 == col2 {
                self.set(col2, row2, arrow_direction);
                self.protect(col2, row2);
            } else {
                let v_in = if row2 > row1 { DIR_UP } else { DIR_DOWN };
                let h_out = if col2 > col1 { DIR_RIGHT } else { DIR_LEFT };
                self.add_dirs(col1, row2, v_in | h_out);

                let (hlo, hhi) = order(col1, col2);
                for x in (hlo + 1)..hhi {
                    self.add_dirs(x, row2, DIR_LEFT | DIR_RIGHT);
                }

                self.set(col2, row2, arrow_direction);
                self.protect(col2, row2);
            }
        }
    }

    // -----------------------------------------------------------------------
    // A* obstacle-aware edge routing
    // -----------------------------------------------------------------------

    /// Route an edge from `(col1, row1)` to `(col2, row2)` using A\* pathfinding
    /// and draw the result on the grid with box-drawing characters.
    ///
    /// The router:
    /// - Treats `NodeBox` cells as impassable hard obstacles.
    /// - Applies a soft penalty (`EDGE_SOFT_COST = 2.0`) when crossing cells
    ///   already occupied by another edge, to reduce clutter.
    /// - Applies a corner penalty (`CORNER_PENALTY = 0.5`) when the routing
    ///   direction changes, to favour straighter paths.
    ///
    /// After finding the path, the method draws it using `─`/`│` for straight
    /// segments and junction characters at corners, placing the arrow tip at
    /// the destination.
    ///
    /// If A\* cannot find any path (e.g. the destination is completely
    /// surrounded by obstacles), the method falls back to the simple Manhattan
    /// routing used by [`Grid::draw_manhattan`].
    ///
    /// # Arguments
    ///
    /// * `col1`, `row1` — source cell (just outside the source node border)
    /// * `col2`, `row2` — destination cell (just outside the destination node
    ///   border, where the arrow tip will be placed)
    /// * `horizontal_first` — hint: prefer horizontal movement first (LR/RL
    ///   flows). A\* may still deviate when obstacles block the preferred path.
    /// * `arrow_direction` — arrow tip character placed at `(col2, row2)`
    ///
    /// # Returns
    ///
    /// The full pixel path as `(col, row)` pairs from source to destination,
    /// including the arrow-tip cell. Returns `None` only when both endpoints
    /// are the same cell.
    pub fn route_edge(
        &mut self,
        col1: usize,
        row1: usize,
        col2: usize,
        row2: usize,
        horizontal_first: bool,
        arrow_direction: char,
    ) -> Option<Vec<(usize, usize)>> {
        // Cost constants.
        //
        // `EDGE_SOFT_COST` is the penalty added when A* enters a cell that
        // a previously-routed edge has already painted. Higher values push
        // edges apart into distinct corridors; lower values let them share
        // trunks. Tuned to favor legibility without sending edges on wide
        // detours — a value of ~4 is enough to split 2-3 parallel edges
        // onto adjacent columns while still accepting a shared trunk when
        // no free column is reachable.
        const EDGE_SOFT_COST: f32 = 4.0;
        const CORNER_PENALTY: f32 = 0.5;
        // 4-directional movement: Right, Down, Left, Up (indices 0..3).
        const DIRS: [(isize, isize); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

        if col1 == col2 && row1 == row2 {
            return None;
        }

        // Manhattan distance heuristic (admissible — never overestimates).
        let h = |c: usize, r: usize| -> f32 { (c.abs_diff(col2) + r.abs_diff(row2)) as f32 };

        // `came_from[row][col]` encodes the direction we arrived from
        // (0–3) or `u8::MAX` for unvisited.  We also store the g_cost.
        let mut g_cost: Vec<Vec<f32>> = vec![vec![f32::INFINITY; self.width]; self.height];
        let mut came_from: Vec<Vec<u8>> = vec![vec![u8::MAX; self.width]; self.height];

        g_cost[row1][col1] = 0.0;

        let mut open: BinaryHeap<AstarNode> = BinaryHeap::new();
        // Preferred initial direction based on `horizontal_first`.
        let start_dir = if horizontal_first { 0u8 } else { 1u8 };
        open.push(AstarNode {
            f_cost: h(col1, row1),
            g_cost: 0.0,
            col: col1,
            row: row1,
            dir: start_dir,
        });

        'outer: while let Some(current) = open.pop() {
            // Skip stale entries (a cheaper path was already found).
            if current.g_cost > g_cost[current.row][current.col] {
                continue;
            }

            if current.col == col2 && current.row == row2 {
                break 'outer;
            }

            for (dir_idx, &(dc, dr)) in DIRS.iter().enumerate() {
                let nc = current.col.wrapping_add_signed(dc);
                let nr = current.row.wrapping_add_signed(dr);
                if nc >= self.width || nr >= self.height {
                    continue;
                }
                // Hard obstacle check.
                if self.obstacles[nr][nc] == Obstacle::NodeBox {
                    // Allow the destination cell even if it is marked as a
                    // node box (the tip sits on the node border).
                    if nc != col2 || nr != row2 {
                        continue;
                    }
                }

                // Base step cost.
                let mut step = 1.0f32;
                // Soft obstacle: crossing an existing edge costs more.
                if self.obstacles[nr][nc] == Obstacle::EdgeOccupied {
                    step += EDGE_SOFT_COST;
                }
                // Corner penalty: direction change from previous step.
                if dir_idx as u8 != current.dir {
                    step += CORNER_PENALTY;
                }

                let new_g = current.g_cost + step;
                if new_g < g_cost[nr][nc] {
                    g_cost[nr][nc] = new_g;
                    // Store the direction of the move INTO (nr, nc) — the
                    // reconstruction walks back by reversing this vector.
                    came_from[nr][nc] = dir_idx as u8;
                    open.push(AstarNode {
                        f_cost: new_g + h(nc, nr),
                        g_cost: new_g,
                        col: nc,
                        row: nr,
                        dir: dir_idx as u8,
                    });
                }
            }
        }

        // Reconstruct path by walking `came_from` backwards from the goal.
        if came_from[row2][col2] == u8::MAX && (col1 != col2 || row1 != row2) {
            // A* found no path — fall back to simple Manhattan routing.
            self.draw_manhattan(col1, row1, col2, row2, horizontal_first, arrow_direction);
            // Return a two-point path for label placement.
            return Some(vec![(col1, row1), (col2, row2)]);
        }

        // Collect waypoints (in reverse order, then reverse).
        let mut path: Vec<(usize, usize)> = Vec::new();
        let mut cc = col2;
        let mut cr = row2;
        path.push((cc, cr));
        while cc != col1 || cr != row1 {
            let dir = came_from[cr][cc];
            if dir == u8::MAX {
                break;
            }
            // `came_from` stores the direction of the move INTO this cell —
            // stepping back means reversing that vector.
            let (dc, dr) = DIRS[dir as usize];
            cc = cc.wrapping_add_signed(-dc);
            cr = cr.wrapping_add_signed(-dr);
            path.push((cc, cr));
        }
        path.reverse();

        // Draw the path on the grid.
        self.draw_routed_path(&path, arrow_direction);
        Some(path)
    }

    /// Overwrite the glyphs of an already-drawn path with a different line style.
    ///
    /// This must be called **after** [`Grid::route_edge`] has drawn the path
    /// using solid glyphs and populated the direction-bit canvas. The method
    /// walks the path (excluding the tip cell, which is handled separately) and
    /// replaces each non-protected cell's glyph according to `style`:
    ///
    /// - [`EdgeLineStyle::Solid`] — no-op (already solid).
    /// - [`EdgeLineStyle::Dotted`] — single-direction cells become `┄`/`┆`;
    ///   multi-direction junction cells are left as solid (see `dotted` module).
    /// - [`EdgeLineStyle::Thick`] — all cells are recomputed from the
    ///   direction-bit canvas using `THICK_DIR_TO_CHAR`.
    ///
    /// The `tip` and `back_tip` cells must be placed by the caller after this
    /// call — they are not in `path_cells` (the path slice passed here should
    /// exclude the terminal arrow cell).
    pub fn overdraw_path_style(&mut self, path_cells: &[(usize, usize)], style: EdgeLineStyle) {
        if style == EdgeLineStyle::Solid {
            return;
        }
        for &(c, r) in path_cells {
            if r >= self.height || c >= self.width {
                continue;
            }
            if self.protected[r][c] {
                continue;
            }
            let bits = self.directions[r][c];
            match style {
                EdgeLineStyle::Solid => {}
                EdgeLineStyle::Dotted => {
                    // Only single-axis cells (pure horizontal or pure vertical)
                    // get dotted glyphs; junctions stay solid to avoid
                    // mismatched box-drawing characters.
                    self.cells[r][c] = match bits {
                        0b0001..=0b0011 => dotted::V,          // any vertical-only
                        0b0100 | 0b1000 | 0b1100 => dotted::H, // any horizontal-only
                        _ => DIR_TO_CHAR[bits as usize],       // junction → stay solid
                    };
                }
                EdgeLineStyle::Thick => {
                    self.cells[r][c] = THICK_DIR_TO_CHAR[bits as usize];
                }
            }
        }
    }

    /// Draw a pre-computed list of `(col, row)` waypoints as box-drawing
    /// chars using the direction-bit canvas.
    ///
    /// For each waypoint, the direction bits pointing toward its path
    /// neighbors (previous and next) are OR'd into the cell; the
    /// direction-to-char table then produces the correct glyph — straight
    /// segments render as `─`/`│`, turns render as corner chars, and
    /// whenever another edge has already painted the same cell the result
    /// merges naturally into a T-junction (`├┤┬┴`) or cross (`┼`).
    ///
    /// The final waypoint is overwritten with the arrow tip and protected
    /// so later edges can't erase it. Each drawn cell is marked as
    /// [`Obstacle::EdgeOccupied`] so subsequent edges pay a higher cost
    /// to cross it.
    fn draw_routed_path(&mut self, path: &[(usize, usize)], tip: char) {
        if path.len() < 2 {
            return;
        }
        let last = path.len() - 1;

        for i in 0..=last {
            let (c, r) = path[i];

            // Mark as edge-occupied so future routes prefer fresh corridors.
            if r < self.height && c < self.width && self.obstacles[r][c] != Obstacle::NodeBox {
                self.obstacles[r][c] = Obstacle::EdgeOccupied;
            }

            if i == last {
                // Arrow tip — fixed glyph, protected against later merges.
                self.set(c, r, tip);
                self.protect(c, r);
                continue;
            }

            let mut bits = 0u8;
            if i > 0 {
                let (pc, pr) = path[i - 1];
                bits |= neighbor_bit(c, r, pc, pr);
            }
            let (nc, nr) = path[i + 1];
            bits |= neighbor_bit(c, r, nc, nr);
            self.add_dirs(c, r, bits);
        }
    }
}

// ---------------------------------------------------------------------------
// Display impl
// ---------------------------------------------------------------------------

impl std::fmt::Display for Grid {
    /// Format the grid as a multi-line string, stripping trailing spaces.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::with_capacity(self.height * (self.width + 1));
        for row in &self.cells {
            let line: String = row.iter().collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        // Remove trailing blank lines
        while out.ends_with("\n\n") {
            out.pop();
        }
        write!(f, "{out}")
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Sort `(a, b)` into `(min, max)` ascending.
fn order(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Return the direction bit that points from cell `(c, r)` toward cell
/// `(nc, nr)` — `DIR_LEFT` if the neighbor is to the left, `DIR_RIGHT` if to
/// the right, etc. Returns `0` if the coordinates are equal or diagonal (the
/// latter should never happen in orthogonal routing).
fn neighbor_bit(c: usize, r: usize, nc: usize, nr: usize) -> u8 {
    if nc < c {
        DIR_LEFT
    } else if nc > c {
        DIR_RIGHT
    } else if nr < r {
        DIR_UP
    } else if nr > r {
        DIR_DOWN
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_set_and_get() {
        let mut g = Grid::new(5, 5);
        g.set(2, 3, 'X');
        assert_eq!(g.get(2, 3), 'X');
        assert_eq!(g.get(0, 0), ' ');
    }

    #[test]
    fn out_of_bounds_ignored() {
        let mut g = Grid::new(3, 3);
        g.set(10, 10, 'X'); // should not panic
        assert_eq!(g.get(10, 10), ' ');
    }

    #[test]
    fn draw_box_corners() {
        let mut g = Grid::new(10, 5);
        g.draw_box(0, 0, 5, 3);
        assert_eq!(g.get(0, 0), '┌');
        assert_eq!(g.get(4, 0), '┐');
        assert_eq!(g.get(0, 2), '└');
        assert_eq!(g.get(4, 2), '┘');
    }

    #[test]
    fn write_text_respects_width() {
        let mut g = Grid::new(20, 3);
        g.write_text(1, 1, "Hello");
        assert_eq!(g.get(1, 1), 'H');
        assert_eq!(g.get(5, 1), 'o');
    }

    #[test]
    fn to_string_strips_trailing_spaces() {
        let g = Grid::new(10, 2);
        let s = g.to_string();
        for line in s.lines() {
            assert!(!line.ends_with(' '));
        }
    }

    #[test]
    fn draw_h_arrow_places_tip() {
        let mut g = Grid::new(20, 3);
        g.draw_h_arrow(2, 1, 8);
        assert_eq!(g.get(8, 1), arrow::RIGHT);
        assert_eq!(g.get(2, 1), '─');
    }

    #[test]
    fn draw_v_arrow_places_tip() {
        let mut g = Grid::new(10, 10);
        g.draw_v_arrow(3, 1, 5);
        assert_eq!(g.get(3, 5), arrow::DOWN);
        assert_eq!(g.get(3, 1), '│');
    }
}
