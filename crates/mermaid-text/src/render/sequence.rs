//! Renderer for Mermaid sequence diagrams.
//!
//! Produces a Unicode box-drawing text representation of a
//! [`SequenceDiagram`].  The layout follows termaid's conventions:
//!
//! - Participant boxes are drawn across the top in declaration order.
//! - A vertical dashed lifeline `┆` runs below each box.
//! - Each message occupies one body row; its label appears on the row above.
//! - Rows are spaced 2 apart (message row + one blank) for readability.
//! - Solid arrows use `─` and `▸`/`◂`; dashed arrows use `┄` and `▸`/`◂`.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::sequence::parse;
//! use mermaid_text::render::sequence::render;
//!
//! let diag = parse("sequenceDiagram\nA->>B: hello").unwrap();
//! let out = render(&diag);
//! assert!(out.contains('A'));
//! assert!(out.contains('B'));
//! assert!(out.contains('┆'));
//! ```

use unicode_width::UnicodeWidthStr;

use crate::sequence::{MessageStyle, SequenceDiagram};

// ---------------------------------------------------------------------------
// Layout constants (mirroring termaid's naming conventions)
// ---------------------------------------------------------------------------

/// Horizontal padding cells added inside each participant box on each side.
const BOX_PAD: usize = 2;

/// Height of the participant box in rows (top border + label + bottom border).
const BOX_HEIGHT: usize = 3;

/// Minimum gap between two adjacent participant *centre* columns.
/// Minimum clearance (in cells) between the inner edges of two adjacent
/// participant boxes. This is the baseline when no message label has to
/// fit inside the gap; labels crossing the gap widen it further.
const MIN_GAP: usize = 6;

/// Rows consumed per regular (non-self) message event (label row + arrow row).
const EVENT_ROW_H: usize = 2;

/// Rows consumed per self-message event. Self-messages render as a two-leg
/// right-loop (`──┐` / `──┘`) plus a label row above, so they need one more
/// row than a regular message to avoid the bottom leg colliding with the
/// next message's label.
const SELF_MSG_ROW_H: usize = 3;

/// Right-pointing solid arrowhead.
const ARROW_RIGHT: char = '▸';
/// Left-pointing solid arrowhead.
const ARROW_LEFT: char = '◂';

/// Solid horizontal line character.
const H_SOLID: char = '─';
/// Dashed horizontal line character.
const H_DASH: char = '┄';

/// Lifeline character.
const LIFELINE: char = '┆';

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

/// A simple character grid for building up the rendered output.
struct Canvas {
    /// Stored in row-major order: `grid[row][col]`.
    grid: Vec<Vec<char>>,
    width: usize,
    height: usize,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            grid: vec![vec![' '; width]; height],
            width,
            height,
        }
    }

    /// Write a single character at `(row, col)`, silently clamping to bounds.
    fn put(&mut self, row: usize, col: usize, ch: char) {
        if row < self.height && col < self.width {
            self.grid[row][col] = ch;
        }
    }

    /// Write a string starting at `(row, col)`.  Characters that would exceed
    /// the canvas width are silently dropped.
    fn put_str(&mut self, row: usize, col: usize, s: &str) {
        let mut c = col;
        for ch in s.chars() {
            if c >= self.width {
                break;
            }
            self.put(row, c, ch);
            // Advance by display width so wide (CJK) characters don't clobber
            // the next cell — for ASCII this is always 1.
            c += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }

    /// Render the grid to a `String` with trailing-space trimming per row.
    fn into_string(self) -> String {
        self.grid
            .iter()
            .map(|row| {
                let s: String = row.iter().collect();
                // Trim trailing spaces for clean output.
                s.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------------------------------------------------------------------------
// Layout computation
// ---------------------------------------------------------------------------

/// Per-participant layout data.
struct ParticipantLayout {
    /// Column of the vertical *centre* of the participant box / lifeline.
    center: usize,
    /// Total width of the participant box (border-to-border).
    box_width: usize,
}

/// Compute column centres and box widths for all participants.
///
/// Column centres are chosen so that:
/// 1. Each box is wide enough to contain its label with `BOX_PAD` on each side.
/// 2. The gap between adjacent centres is at least `MIN_GAP`.
/// 3. The gap is widened further when a message label crossing that gap
///    would not otherwise fit.
fn compute_layout(diag: &SequenceDiagram) -> Vec<ParticipantLayout> {
    let n = diag.participants.len();
    if n == 0 {
        return Vec::new();
    }

    // Minimum box width = label display width + 2 * BOX_PAD + 2 (borders).
    let box_widths: Vec<usize> = diag
        .participants
        .iter()
        .map(|p| {
            let label_w = p.label.width();
            // Ensure the box is at least wide enough for its label.
            (label_w + 2 * BOX_PAD + 2).max(8)
        })
        .collect();

    // Per-gap minimum width driven by message labels that cross that gap.
    // gap_mins[i] is the minimum distance between centres of participant i and i+1.
    let mut gap_mins = vec![MIN_GAP; n.saturating_sub(1)];

    for msg in &diag.messages {
        let Some(si) = diag.participant_index(&msg.from) else {
            continue;
        };
        let Some(ti) = diag.participant_index(&msg.to) else {
            continue;
        };
        if si == ti {
            continue; // self-message; handled separately
        }
        let lo = si.min(ti);
        let hi = si.max(ti);
        let spans = hi - lo;
        // Label needs label_len + 6 cells of horizontal space.
        let label_need = msg.text.width() + 6;
        let per_gap = label_need.div_ceil(spans);
        for slot in gap_mins.iter_mut().take(hi).skip(lo) {
            *slot = (*slot).max(per_gap);
        }
    }

    // Build centre positions cumulatively from the left.
    //
    // `gap_mins[i]` is the minimum *clearance* between the inner edges of
    // box i and box i+1 (not a centre-to-centre distance) so that wide
    // participant labels don't cause boxes to visually touch. Converting
    // to centre-to-centre: add half the previous box's width and half the
    // current box's width.
    let left_margin = box_widths[0] / 2 + 1;
    let mut layouts = Vec::with_capacity(n);
    let mut prev_center = left_margin;

    for i in 0..n {
        let center = if i == 0 {
            left_margin
        } else {
            prev_center
                + box_widths[i - 1] / 2
                + gap_mins[i - 1]
                + box_widths[i] / 2
        };
        layouts.push(ParticipantLayout {
            center,
            box_width: box_widths[i],
        });
        prev_center = center;
    }

    layouts
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// Draw a single-line participant box centered on `cx` in row 0.
///
/// ```text
/// ┌──────┐
/// │ Alice│
/// └──────┘
/// ```
fn draw_participant_box(canvas: &mut Canvas, cx: usize, box_width: usize, label: &str) {
    let left = cx.saturating_sub(box_width / 2);
    let right = left + box_width - 1; // inclusive column of right border

    // Top border
    canvas.put(0, left, '┌');
    for c in (left + 1)..right {
        canvas.put(0, c, '─');
    }
    canvas.put(0, right, '┐');

    // Label row — center the label inside the box.
    let label_w = label.width();
    let inner_w = box_width.saturating_sub(2); // space between borders
    let label_start = left + 1 + (inner_w.saturating_sub(label_w)) / 2;
    canvas.put(1, left, '│');
    canvas.put_str(1, label_start, label);
    canvas.put(1, right, '│');

    // Bottom border
    canvas.put(2, left, '└');
    for c in (left + 1)..right {
        canvas.put(2, c, '─');
    }
    canvas.put(2, right, '┘');
}

/// Draw the lifeline `┆` column from row `start` to row `end` (inclusive).
fn draw_lifeline(canvas: &mut Canvas, cx: usize, start: usize, end: usize) {
    for r in start..=end {
        // Only overwrite spaces — don't clobber arrow characters.
        if canvas.grid[r][cx] == ' ' {
            canvas.put(r, cx, LIFELINE);
        }
    }
}

/// Draw a horizontal message arrow between two column centres on `row`.
/// The label is placed on `row - 1` (above the arrow).
fn draw_message(
    canvas: &mut Canvas,
    src_cx: usize,
    tgt_cx: usize,
    row: usize,
    text: &str,
    style: MessageStyle,
) {
    let going_right = tgt_cx > src_cx;
    let left = src_cx.min(tgt_cx);
    let right = src_cx.max(tgt_cx);
    let h_char = if style.is_dashed() { H_DASH } else { H_SOLID };

    // Draw horizontal line between the two lifeline columns (exclusive of
    // the endpoint columns themselves, which are either arrowheads or line
    // characters).
    for c in (left + 1)..right {
        canvas.put(row, c, h_char);
    }

    if style.has_arrow() {
        if going_right {
            canvas.put(row, left, h_char); // source side: extend line
            canvas.put(row, right, ARROW_RIGHT);
        } else {
            canvas.put(row, left, ARROW_LEFT);
            canvas.put(row, right, h_char);
        }
    } else {
        // No arrowhead — line extends to both endpoints.
        canvas.put(row, left, h_char);
        canvas.put(row, right, h_char);
    }

    // Label above the arrow (termaid convention).
    if !text.is_empty() && row > 0 {
        // Place label starting 2 columns right of the leftmost column so it
        // sits clearly over the arrow shaft.
        let label_col = left + 2;
        canvas.put_str(row - 1, label_col, text);
    }
}

/// Draw a self-message loop to the right of the lifeline column.
///
/// ```text
///  label
/// ┆──┐
/// ◂──┘
/// ```
fn draw_self_message(canvas: &mut Canvas, cx: usize, row: usize, text: &str, style: MessageStyle) {
    let h_char = if style.is_dashed() { H_DASH } else { H_SOLID };
    // TODO: Self-messages with dashed line style use the same box shape;
    // only the horizontal segments change character.
    let loop_w = text.width().max(4) + 4;
    let right = cx + loop_w;

    // Top horizontal leg
    for c in (cx + 1)..right {
        canvas.put(row, c, h_char);
    }
    canvas.put(row, right, '┐');

    // Bottom horizontal leg (row + 1)
    for c in (cx + 1)..right {
        canvas.put(row + 1, c, h_char);
    }
    canvas.put(row + 1, right, '┘');

    // Arrowhead or plain line at the return point.
    if style.has_arrow() {
        canvas.put(row + 1, cx, ARROW_LEFT);
    } else {
        canvas.put(row + 1, cx, h_char);
    }

    // Label above.
    if !text.is_empty() && row > 0 {
        canvas.put_str(row - 1, cx + 2, text);
    }
}

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render a [`SequenceDiagram`] to a Unicode string.
///
/// Returns an empty string if the diagram has no participants.
///
/// # Examples
///
/// ```
/// use mermaid_text::parser::sequence::parse;
/// use mermaid_text::render::sequence::render;
///
/// let diag = parse("sequenceDiagram\nA->>B: hello\nB-->>A: world").unwrap();
/// let out = render(&diag);
/// assert!(out.contains("hello"));
/// assert!(out.contains("world"));
/// assert!(out.contains('┆'));
/// ```
pub fn render(diag: &SequenceDiagram) -> String {
    let n = diag.participants.len();
    if n == 0 {
        return String::new();
    }

    let layouts = compute_layout(diag);

    // Determine canvas dimensions.
    // Header: rows 0-2 (BOX_HEIGHT = 3).
    // Body: one row per message slot, each slot is EVENT_ROW_H rows.
    // We need an extra leading row per message for the label above the arrow
    // so the body starts at row BOX_HEIGHT + 1 (the +1 is the label row for
    // the first message).
    let num_messages = diag.messages.len();

    // Total body rows: each message needs EVENT_ROW_H rows, but we also need
    // a label row *above* the first arrow, so:
    //   body_rows = 1 (initial spacer/label row) + num_messages * EVENT_ROW_H
    let body_rows = if num_messages == 0 {
        2 // just lifeline + blank
    } else {
        // Budget one row per message slot; self-messages need an extra
        // row each for their loop's second leg.
        let self_msg_count = diag
            .messages
            .iter()
            .filter(|m| m.from == m.to)
            .count();
        let regular_count = num_messages - self_msg_count;
        1 + regular_count * EVENT_ROW_H + self_msg_count * SELF_MSG_ROW_H
    };

    let height = BOX_HEIGHT + body_rows;

    // Canvas width: rightmost participant box right edge + 1 margin.
    let last = &layouts[n - 1];
    // For self-messages on the last participant, add extra width.
    let self_msg_extra = diag
        .messages
        .iter()
        .filter(|m| {
            diag.participant_index(&m.from) == diag.participant_index(&m.to)
                && diag.participant_index(&m.from) == Some(n - 1)
        })
        .map(|m| m.text.width() + 6)
        .max()
        .unwrap_or(0);
    let width = last.center + last.box_width / 2 + 2 + self_msg_extra;

    let mut canvas = Canvas::new(width, height);

    // 1. Draw participant boxes.
    for (i, p) in diag.participants.iter().enumerate() {
        draw_participant_box(
            &mut canvas,
            layouts[i].center,
            layouts[i].box_width,
            &p.label,
        );
    }

    // 2. Draw lifelines from bottom of boxes to end of canvas.
    let lifeline_start = BOX_HEIGHT; // row 3 (0-indexed)
    let lifeline_end = height - 1;
    for layout in &layouts {
        draw_lifeline(&mut canvas, layout.center, lifeline_start, lifeline_end);
    }

    // 3. Draw messages.
    //
    // Each non-self message consumes `EVENT_ROW_H` rows (label row + arrow
    // row + 1 blank spacer, with EVENT_ROW_H=2 accounting for label+arrow).
    // Self-messages span `SELF_MSG_ROW_H` rows because their loop draws a
    // top leg and a bottom leg — placing the next message's label on
    // `row+1` would overlap the self-loop's bottom leg.
    let mut arrow_row = BOX_HEIGHT + 1;
    for msg in &diag.messages {
        let Some(si) = diag.participant_index(&msg.from) else {
            continue;
        };
        let Some(ti) = diag.participant_index(&msg.to) else {
            continue;
        };

        if si == ti {
            draw_self_message(
                &mut canvas,
                layouts[si].center,
                arrow_row,
                &msg.text,
                msg.style,
            );
            arrow_row += SELF_MSG_ROW_H;
        } else {
            draw_message(
                &mut canvas,
                layouts[si].center,
                layouts[ti].center,
                arrow_row,
                &msg.text,
                msg.style,
            );
            arrow_row += EVENT_ROW_H;
        }
    }

    canvas.into_string()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::sequence::parse;

    #[test]
    fn render_produces_participant_boxes() {
        let diag = parse("sequenceDiagram\nparticipant A as Alice\nparticipant B as Bob").unwrap();
        let out = render(&diag);
        assert!(out.contains("Alice"), "missing Alice in:\n{out}");
        assert!(out.contains("Bob"), "missing Bob in:\n{out}");
        // Boxes use corner characters.
        assert!(out.contains('┌'), "no box corner in:\n{out}");
    }

    #[test]
    fn render_draws_lifelines() {
        let diag = parse("sequenceDiagram\nA->>B: hi").unwrap();
        let out = render(&diag);
        assert!(out.contains(LIFELINE), "no lifeline char in:\n{out}");
    }

    #[test]
    fn render_solid_arrow() {
        let diag = parse("sequenceDiagram\nA->>B: go").unwrap();
        let out = render(&diag);
        assert!(out.contains(ARROW_RIGHT), "no solid arrowhead in:\n{out}");
    }

    #[test]
    fn render_dashed_arrow() {
        let diag = parse("sequenceDiagram\nA-->>B: back").unwrap();
        let out = render(&diag);
        assert!(out.contains(H_DASH), "no dashed glyph in:\n{out}");
    }

    #[test]
    fn render_message_text_appears() {
        let diag = parse("sequenceDiagram\nA->>B: Hello Bob").unwrap();
        let out = render(&diag);
        assert!(out.contains("Hello Bob"), "missing message text in:\n{out}");
    }

    #[test]
    fn render_message_order_top_to_bottom() {
        let diag = parse("sequenceDiagram\nA->>B: first\nB->>A: second").unwrap();
        let out = render(&diag);
        let first_row = out
            .lines()
            .position(|l| l.contains("first"))
            .expect("'first' not found");
        let second_row = out
            .lines()
            .position(|l| l.contains("second"))
            .expect("'second' not found");
        assert!(
            first_row < second_row,
            "'first' should appear above 'second':\n{out}"
        );
    }

    #[test]
    fn render_empty_diagram_is_empty_string() {
        let diag = crate::sequence::SequenceDiagram::default();
        assert_eq!(render(&diag), "");
    }
}
