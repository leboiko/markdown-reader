//! 2D character grid used for building the final text output.
//!
//! The grid stores one `char` per cell. All drawing operations write
//! directly into the grid; the final string is produced by converting the
//! grid to a `String` via its [`std::fmt::Display`] implementation.

use unicode_width::UnicodeWidthChar;

// Box-drawing character sets
/// Rectangle corners and sides.
mod rect {
    pub const TL: char = '┌';
    pub const TR: char = '┐';
    pub const BL: char = '└';
    pub const BR: char = '┘';
    pub const H: char = '─';
    pub const V: char = '│';
    pub const T_DOWN: char = '┬'; // T junction, branch down
    pub const T_UP: char = '┴'; // T junction, branch up
    pub const T_RIGHT: char = '├'; // T junction, branch right
    pub const T_LEFT: char = '┤'; // T junction, branch left
    pub const CROSS: char = '┼';
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
            width,
            height,
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

    /// Draw a diamond shape centered at `(cx, cy)` with half-widths `hw` and `hh`.
    ///
    /// The diamond is drawn using `/` and `\` characters on the diagonal edges
    /// and `─` on any flat portions. The height is forced to an odd number
    /// so there is a clear centre row.
    ///
    /// `w` and `h` are the total bounding-box dimensions.
    pub fn draw_diamond(&mut self, col: usize, row: usize, w: usize, h: usize) {
        if w < 3 || h < 3 {
            return;
        }
        // Draw the diamond as four lines from the midpoints of each side.
        // Top midpoint: (col + w/2, row)
        // Bottom midpoint: (col + w/2, row + h - 1)
        // Left midpoint: (col, row + h/2)
        // Right midpoint: (col + w - 1, row + h/2)
        let mid_x = col + w / 2;
        let mid_y = row + h / 2;

        // Top edge and bottom edge (horizontal dashes if wide)
        self.set(mid_x, row, '▲');
        self.set(mid_x, row + h - 1, '▼');
        self.set(col, mid_y, '◁');
        self.set(col + w - 1, mid_y, '▷');

        // Draw the four diagonal lines
        // Top-left diagonal: from (mid_x, row) to (col, mid_y)
        self.draw_diagonal(mid_x, row, col, mid_y);
        // Top-right diagonal: from (mid_x, row) to (col+w-1, mid_y)
        self.draw_diagonal(mid_x, row, col + w - 1, mid_y);
        // Bottom-left diagonal: from (col, mid_y) to (mid_x, row+h-1)
        self.draw_diagonal(col, mid_y, mid_x, row + h - 1);
        // Bottom-right diagonal: from (col+w-1, mid_y) to (mid_x, row+h-1)
        self.draw_diagonal(col + w - 1, mid_y, mid_x, row + h - 1);
    }

    /// Draw a diagonal line between two points using `/` or `\` characters.
    fn draw_diagonal(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        // Use signed arithmetic to determine direction
        let dx = x2 as isize - x1 as isize;
        let dy = y2 as isize - y1 as isize;
        let steps = dx.unsigned_abs().max(dy.unsigned_abs());
        if steps == 0 {
            return;
        }

        for s in 1..steps {
            let x = (x1 as isize + dx * s as isize / steps as isize) as usize;
            let y = (y1 as isize + dy * s as isize / steps as isize) as usize;
            // '\' when moving right+down or left+up; '/' otherwise
            let ch = if (dx > 0 && dy > 0) || (dx < 0 && dy < 0) {
                '\\'
            } else {
                '/'
            };
            self.set(x, y, ch);
        }
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
            // Respect junction characters already on the grid
            let existing = self.get(x, row);
            let ch = merge_h_line(existing);
            self.set(x, row, ch);
        }
        self.set(col2, row, arrow::RIGHT);
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
            let existing = self.get(col, y);
            let ch = merge_v_line(existing);
            self.set(col, y, ch);
        }
        self.set(col, row2, arrow::DOWN);
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
            // Horizontal segment from (col1, row1) to (col2, row1)
            if col1 != col2 {
                self.draw_h_line(col1, row1, col2);
            }
            // Corner
            if col1 != col2 && row1 != row2 {
                let existing = self.get(col2, row1);
                self.set(col2, row1, merge_corner_h_v(existing));
            }
            // Vertical segment from (col2, row1) to (col2, row2) with tip
            if row1 != row2 {
                self.draw_v_line_with_tip(col2, row1, row2, arrow_direction);
            } else {
                // Pure horizontal — place tip at end
                self.set(col2, row2, arrow_direction);
            }
        } else {
            // Vertical segment from (col1, row1) to (col1, row2)
            if row1 != row2 {
                self.draw_v_line(col1, row1, row2);
            }
            // Corner
            if col1 != col2 && row1 != row2 {
                let existing = self.get(col1, row2);
                self.set(col1, row2, merge_corner_v_h(existing));
            }
            // Horizontal segment from (col1, row2) to (col2, row2) with tip
            if col1 != col2 {
                self.draw_h_line_with_tip(col1, row2, col2, arrow_direction);
            } else {
                self.set(col2, row2, arrow_direction);
            }
        }
    }

    // -- internal helpers ---------------------------------------------------

    /// Draw a plain horizontal line (no tip) from col1 to col2 (exclusive).
    fn draw_h_line(&mut self, col1: usize, row: usize, col2: usize) {
        let (lo, hi) = if col1 <= col2 {
            (col1, col2)
        } else {
            (col2, col1)
        };
        for x in lo..hi {
            let existing = self.get(x, row);
            self.set(x, row, merge_h_line(existing));
        }
    }

    /// Draw a plain vertical line (no tip) from row1 to row2 (exclusive).
    fn draw_v_line(&mut self, col: usize, row1: usize, row2: usize) {
        let (lo, hi) = if row1 <= row2 {
            (row1, row2)
        } else {
            (row2, row1)
        };
        for y in lo..hi {
            let existing = self.get(col, y);
            self.set(col, y, merge_v_line(existing));
        }
    }

    /// Draw a horizontal line with an arrow tip at the destination end.
    fn draw_h_line_with_tip(&mut self, col1: usize, row: usize, col2: usize, tip: char) {
        let (lo, hi) = if col1 <= col2 {
            (col1, col2)
        } else {
            (col2, col1)
        };
        for x in lo..hi {
            let existing = self.get(x, row);
            self.set(x, row, merge_h_line(existing));
        }
        self.set(col2, row, tip);
    }

    /// Draw a vertical line with an arrow tip at the destination end.
    fn draw_v_line_with_tip(&mut self, col: usize, row1: usize, row2: usize, tip: char) {
        let (lo, hi) = if row1 <= row2 {
            (row1, row2)
        } else {
            (row2, row1)
        };
        for y in lo..hi {
            let existing = self.get(col, y);
            self.set(col, y, merge_v_line(existing));
        }
        self.set(col, row2, tip);
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
// Merge helpers — prevent overwriting structural characters
// ---------------------------------------------------------------------------

/// Choose the correct character when writing a horizontal line segment,
/// merging with any existing box-drawing character.
fn merge_h_line(existing: char) -> char {
    match existing {
        '│' => rect::CROSS,
        rect::T_DOWN | rect::T_UP | rect::CROSS => existing,
        _ => rect::H,
    }
}

/// Choose the correct character when writing a vertical line segment.
fn merge_v_line(existing: char) -> char {
    match existing {
        '─' => rect::CROSS,
        rect::T_RIGHT | rect::T_LEFT | rect::CROSS => existing,
        _ => rect::V,
    }
}

/// Merge a corner character where a horizontal line turns into a vertical one.
fn merge_corner_h_v(existing: char) -> char {
    match existing {
        '─' => rect::T_DOWN,
        '│' => rect::T_LEFT,
        ' ' | arrow::RIGHT | arrow::DOWN | arrow::LEFT | arrow::UP => rect::T_DOWN,
        _ => existing,
    }
}

/// Merge a corner character where a vertical line turns into a horizontal one.
fn merge_corner_v_h(existing: char) -> char {
    match existing {
        '│' => rect::T_RIGHT,
        '─' => rect::T_UP,
        ' ' | arrow::RIGHT | arrow::DOWN | arrow::LEFT | arrow::UP => rect::T_RIGHT,
        _ => existing,
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
