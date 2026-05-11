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

use crate::sequence::{
    AutonumberState, Block, BlockKind, MessageStyle, NoteAnchor, SequenceDiagram,
};

// ---------------------------------------------------------------------------
// Layout constants (mirroring termaid's naming conventions)
// ---------------------------------------------------------------------------

/// Horizontal padding cells added inside each participant box on each side.
const BOX_PAD: usize = 2;

/// Height of the participant box in rows (top border + label + bottom border).
const BOX_HEIGHT: usize = 3;

/// Minimum gap between two adjacent participant *centre* columns.
/// Minimum clearance (in cells) between the inner edges of two adjacent
/// participant boxes. Baseline when no message label crosses the gap;
/// labels widen it further via [`LABEL_PADDING`].
const MIN_GAP: usize = 2;

/// Cells added to a message label's width when computing how much gap
/// space that label needs. Covers one cell of visual padding at the left
/// of the label and one at the right of the arrow tip.
const LABEL_PADDING: usize = 2;

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

// Activation bar — full-block glyph, drawn ACTIVATION_BAR_WIDTH cells
// wide centred on the lifeline so the active span reads as a "filled
// rectangle" matching Mermaid's SVG output rather than a thin heavy
// line. The bar overlays the dashed lifeline `┆` and skips cells
// already holding arrow/junction glyphs from messages.
const ACTIVATION_BAR: char = '█';
const ACTIVATION_BAR_WIDTH: usize = 2;

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
        // Label needs `label_width + LABEL_PADDING` cells of clearance along
        // its arrow; divide across the spans the arrow crosses.
        let label_need = msg.text.width() + LABEL_PADDING;
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
            prev_center + box_widths[i - 1] / 2 + gap_mins[i - 1] + box_widths[i] / 2
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

/// Draw a single-line participant box centered on `cx`, with the top
/// border on `top_row`. The box occupies three consecutive rows:
/// `top_row` (top border), `top_row + 1` (label), `top_row + 2`
/// (bottom border).
///
/// ```text
/// ┌──────┐
/// │ Alice│
/// └──────┘
/// ```
///
/// Used twice per render: once at `top_row = 0` for the header and
/// once at `top_row = canvas.height - BOX_HEIGHT` for the mirrored
/// footer (matches Mermaid's convention of bracketing lifelines).
fn draw_participant_box(
    canvas: &mut Canvas,
    cx: usize,
    box_width: usize,
    label: &str,
    top_row: usize,
) {
    let left = cx.saturating_sub(box_width / 2);
    let right = left + box_width - 1; // inclusive column of right border
    let label_row = top_row + 1;
    let bottom_row = top_row + 2;

    // Top border
    canvas.put(top_row, left, '┌');
    for c in (left + 1)..right {
        canvas.put(top_row, c, '─');
    }
    canvas.put(top_row, right, '┐');

    // Label row — center the label inside the box.
    let label_w = label.width();
    let inner_w = box_width.saturating_sub(2); // space between borders
    let label_start = left + 1 + (inner_w.saturating_sub(label_w)) / 2;
    canvas.put(label_row, left, '│');
    canvas.put_str(label_row, label_start, label);
    canvas.put(label_row, right, '│');

    // Bottom border
    canvas.put(bottom_row, left, '└');
    for c in (left + 1)..right {
        canvas.put(bottom_row, c, '─');
    }
    canvas.put(bottom_row, right, '┘');
}

/// Draw a multi-line note box on the canvas with rounded corners.
///
/// `left` and `right` are the inclusive column bounds; `text` is the
/// note's content (one logical line per `\n`). Box height is
/// `text.lines().count() + 2` (top border + content rows + bottom
/// border). Rounded corners (`╭ ╮ ╰ ╯`) distinguish notes from
/// participant header boxes (which use square `┌ ┐ └ ┘` corners).
///
/// Lifelines are drawn in an earlier pass; the note's borders
/// naturally overwrite the dashed `┆` glyphs in the columns it
/// occupies, which reads as the note "covering" the lifeline at
/// that point.
fn draw_note_box(canvas: &mut Canvas, left: usize, right: usize, row: usize, text: &str) {
    if right < left {
        return;
    }
    let lines: Vec<&str> = text.lines().collect();
    let height = lines.len() + 2;

    // Top border.
    canvas.put(row, left, '╭');
    for c in (left + 1)..right {
        canvas.put(row, c, '─');
    }
    canvas.put(row, right, '╮');

    // Content rows. Lifelines (`┆`) drawn in an earlier pass may
    // intrude on the interior columns; clear the interior to spaces
    // first so the note reads as a solid box rather than a frame
    // with dashed lines bleeding through.
    let inner_left = left + 2; // 1 cell padding inside the border
    for (i, line) in lines.iter().enumerate() {
        let r = row + 1 + i;
        canvas.put(r, left, '│');
        for c in (left + 1)..right {
            canvas.put(r, c, ' ');
        }
        canvas.put(r, right, '│');
        canvas.put_str(r, inner_left, line);
    }

    // Bottom border.
    let bottom = row + height - 1;
    canvas.put(bottom, left, '╰');
    for c in (left + 1)..right {
        canvas.put(bottom, c, '─');
    }
    canvas.put(bottom, right, '╯');
}

/// Compute the inclusive `(left_col, right_col)` for a note box
/// based on its anchor and the current participant layouts.
///
/// Returns `None` when the anchor names a participant that doesn't
/// exist in the diagram (the parser auto-creates participants
/// referenced by messages, but a note can name a never-mentioned id).
fn note_columns(
    anchor: &NoteAnchor,
    layouts: &[ParticipantLayout],
    diag: &SequenceDiagram,
    text_w: usize,
) -> Option<(usize, usize)> {
    // Box width = text + 2 cells padding each side + 2 borders.
    let box_w = text_w + 4;
    match anchor {
        NoteAnchor::LeftOf(id) => {
            let i = diag.participant_index(id)?;
            let right = layouts[i].center.saturating_sub(2);
            let left = right.saturating_sub(box_w.saturating_sub(1));
            Some((left, right))
        }
        NoteAnchor::RightOf(id) => {
            let i = diag.participant_index(id)?;
            let left = layouts[i].center + 2;
            Some((left, left + box_w - 1))
        }
        NoteAnchor::Over(id) => {
            let i = diag.participant_index(id)?;
            let center = layouts[i].center;
            let left = center.saturating_sub(box_w / 2);
            Some((left, left + box_w - 1))
        }
        NoteAnchor::OverPair(a, b) => {
            let i = diag.participant_index(a)?;
            let j = diag.participant_index(b)?;
            let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
            let span_left = layouts[lo].center;
            let span_right = layouts[hi].center;
            let span_w = span_right - span_left + 1;
            // Widen the box to span both anchors + padding; if the
            // text is wider than the span, the box extends to fit.
            let needed_w = box_w.max(span_w + 2);
            let centre = (span_left + span_right) / 2;
            let left = centre.saturating_sub(needed_w / 2);
            Some((left, left + needed_w - 1))
        }
    }
}

/// Compute the maximum display width across the lines of `text`.
fn max_line_width(text: &str) -> usize {
    text.lines().map(|l| l.width()).max().unwrap_or(0)
}

/// Word-wrap `text` so that no line exceeds `budget` display cells.
///
/// Each pre-existing `\n` in `text` is an authoritative break (from the user's
/// `<br>` or `<br/>`) and is always preserved exactly — the lines it produces
/// are never re-joined and re-split. Within each such segment, words are packed
/// greedily left-to-right. A word that exceeds `budget` by itself is left on
/// its own line and the canvas-widening pass will handle it.
///
/// Returns the wrapped string. If every line already fits within `budget`,
/// the input is returned unchanged.
fn wrap_note_text(text: &str, budget: usize) -> String {
    if budget == 0 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for (seg_idx, segment) in text.split('\n').enumerate() {
        if seg_idx > 0 {
            out.push('\n');
        }
        // If the segment already fits, emit it unchanged.
        if segment.width() <= budget {
            out.push_str(segment);
            continue;
        }
        // Greedy word-wrap within this segment.
        let mut current_w = 0usize;
        let mut first_word = true;
        for word in segment.split_ascii_whitespace() {
            let w = word.width();
            if first_word {
                out.push_str(word);
                current_w = w;
                first_word = false;
            } else if current_w + 1 + w <= budget {
                out.push(' ');
                out.push_str(word);
                current_w += 1 + w;
            } else {
                // Word doesn't fit on current line — start a new one.
                out.push('\n');
                out.push_str(word);
                current_w = w;
            }
        }
        // Segment was non-empty whitespace-only (edge case).
        if first_word {
            out.push_str(segment);
        }
    }
    out
}

/// Compute the text-width budget for a note given its anchor.
///
/// Budget = available display columns for note content (excluding the 2-cell
/// border and 2-cell padding on each side, i.e., box_width - 4).
///
/// Policy:
/// - `LeftOf(X)`: everything to the left of the anchor's lifeline, minus the
///   2-cell gap between box right edge and the lifeline.
/// - `RightOf(X)`: everything to the right of the anchor, clipped to a
///   comfortable 30 cells so the box doesn't run off-screen on typical terms.
/// - `Over(X)`: 40 cells (Mermaid-ish default for single-participant notes).
/// - `OverPair(X, Y)`: the span between the two centres, minus box padding.
fn note_budget(
    anchor: &NoteAnchor,
    layouts: &[ParticipantLayout],
    diag: &SequenceDiagram,
) -> usize {
    const RIGHT_OF_BUDGET: usize = 30;
    const OVER_SINGLE_BUDGET: usize = 40;
    const BOX_OVERHEAD: usize = 4; // 2 borders + 2 padding cells on each side = 4 per side? No: overhead is border(1)+pad(1) on each side = 4 total

    match anchor {
        NoteAnchor::LeftOf(id) => {
            let Some(i) = diag.participant_index(id) else {
                return OVER_SINGLE_BUDGET;
            };
            let right_edge = layouts[i].center.saturating_sub(2);
            right_edge.saturating_sub(BOX_OVERHEAD)
        }
        NoteAnchor::RightOf(_) => RIGHT_OF_BUDGET,
        NoteAnchor::Over(_) => OVER_SINGLE_BUDGET,
        NoteAnchor::OverPair(a, b) => {
            let Some(i) = diag.participant_index(a) else {
                return OVER_SINGLE_BUDGET;
            };
            let Some(j) = diag.participant_index(b) else {
                return OVER_SINGLE_BUDGET;
            };
            let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
            let span_w = layouts[hi].center.saturating_sub(layouts[lo].center) + 1;
            span_w.saturating_sub(BOX_OVERHEAD).max(OVER_SINGLE_BUDGET)
        }
    }
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

    // Top leg: `├──────┐`. `├` at the lifeline column makes the branch-off
    // from the lifeline visually explicit (otherwise the dashed `┆` lifeline
    // cell reads as disconnected from the solid horizontal). Horizontal
    // segment fills the rest, `┐` is the top-right corner.
    canvas.put(row, cx, '├');
    for c in (cx + 1)..right {
        canvas.put(row, c, h_char);
    }
    canvas.put(row, right, '┐');

    // Bottom leg: `├◂─────┘`. T-junction at the lifeline, arrow tip
    // immediately inside the loop so the return-to-sender direction reads
    // clearly, then horizontal segment, then `┘` corner. For plain-line
    // (no-arrow) style the arrow slot becomes another horizontal char.
    canvas.put(row + 1, cx, '├');
    if style.has_arrow() {
        canvas.put(row + 1, cx + 1, ARROW_LEFT);
    } else {
        canvas.put(row + 1, cx + 1, h_char);
    }
    for c in (cx + 2)..right {
        canvas.put(row + 1, c, h_char);
    }
    canvas.put(row + 1, right, '┘');

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
        let self_msg_count = diag.messages.iter().filter(|m| m.from == m.to).count();
        let regular_count = num_messages - self_msg_count;
        1 + regular_count * EVENT_ROW_H + self_msg_count * SELF_MSG_ROW_H
    };

    // Pre-compute wrapped note text for every note. Two-pass approach:
    //   Pass 1 (here): wrap each note's text to its anchor budget, record the
    //           required canvas width for unbreakable words that exceed the
    //           budget, and compute the wrapped line count for height budgeting.
    //   Pass 2 (draw loop): use the wrapped texts when calling draw_note_box.
    let note_wrapped: Vec<String> = diag
        .notes
        .iter()
        .map(|note| {
            let budget = note_budget(&note.anchor, &layouts, diag);
            wrap_note_text(&note.text, budget)
        })
        .collect();

    // Notes consume their own rows in the message stream. Sum them
    // into the height budget so `Canvas::new` allocates enough space.
    // Use the wrapped text line count so the canvas is tall enough.
    let note_rows: usize = note_wrapped
        .iter()
        .map(|t| t.lines().count().max(1) + 3)
        .sum();

    // Block frames add 2 rows per border and per branch divider (1 for
    // the glyph itself + 1 spacer below) so adjacent message labels
    // don't land on the same row as a border. Total per block:
    //   2 (top) + 2 (bottom) + 2 * (extra branches).
    let block_rows: usize = diag
        .blocks
        .iter()
        .map(|b| 4 + 2 * b.branches.len().saturating_sub(1))
        .sum();

    // Mirror the header participant boxes at the bottom of the canvas
    // (Mermaid convention — lifelines are bracketed top *and* bottom).
    // Add another `BOX_HEIGHT` rows for the footer.
    let height = BOX_HEIGHT + body_rows + note_rows + block_rows + BOX_HEIGHT;

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
    let participant_width = last.center + last.box_width / 2 + 2 + self_msg_extra;

    // Widen the canvas when a note's wrapped text + box overhead would exceed
    // the participant span. This handles unbreakable words (a word wider than
    // the wrapping budget) that couldn't be split by `wrap_note_text`.
    let note_required_width: usize = diag
        .notes
        .iter()
        .zip(note_wrapped.iter())
        .filter_map(|(note, wrapped)| {
            let text_w = max_line_width(wrapped);
            // box_w = text_w + 4 (border + padding on each side)
            let (_l, r) = note_columns(&note.anchor, &layouts, diag, text_w)?;
            // right edge of the note box + 1 margin
            Some(r + 2)
        })
        .max()
        .unwrap_or(0);

    let width = participant_width.max(note_required_width);

    let mut canvas = Canvas::new(width, height);

    // 1. Draw participant boxes — header (top) and footer (bottom)
    //    mirror, matching Mermaid's bracketed-lifeline convention.
    let footer_top = height - BOX_HEIGHT;
    for (i, p) in diag.participants.iter().enumerate() {
        let cx = layouts[i].center;
        let w = layouts[i].box_width;
        draw_participant_box(&mut canvas, cx, w, &p.label, 0);
        draw_participant_box(&mut canvas, cx, w, &p.label, footer_top);
    }

    // 2. Draw lifelines between the header and footer boxes — they
    //    must terminate one row above the footer's top border so the
    //    box outline reads as a clean bracket (lifeline glyphs would
    //    otherwise punch through the `┌────┐`).
    let lifeline_start = BOX_HEIGHT; // row right below the header
    let lifeline_end = footer_top.saturating_sub(1);
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
    let mut autonumber = AutonumberState::Off;
    let mut autonumber_cursor = 0usize;

    // Captured arrow row for each message, indexed by message position.
    // Used by the activation-bar overlay pass to translate
    // `Activation::start_message` / `end_message` (message indices) into
    // canvas rows. For self-messages we store the top-leg row so the bar
    // naturally covers both legs.
    let mut message_arrow_rows: Vec<usize> = Vec::with_capacity(num_messages);

    // Block frame events to insert at each message-index boundary. At
    // boundary `B`:
    //   - bottom borders of any block ending at message `B - 1`
    //   - top borders of any block starting at message `B`
    //   - dividers for any branch (other than the first) starting at `B`
    // These advance `arrow_row` and capture each event's row so the
    // post-loop overlay pass knows where to draw frames.
    let num_blocks = diag.blocks.len();
    let mut block_top_rows: Vec<usize> = vec![0; num_blocks];
    let mut block_bottom_rows: Vec<usize> = vec![0; num_blocks];
    let mut branch_divider_rows: Vec<Vec<usize>> = diag
        .blocks
        .iter()
        .map(|b| vec![0usize; b.branches.len()])
        .collect();

    // Helper closure: process all block events at message-index `pos`,
    // advancing `arrow_row` by 2 per event (1 for the border/divider
    // glyph row + 1 spacer below it so adjacent message labels don't
    // collide). The border row is the *first* of the two; the spacer
    // is implicitly the second. Inner blocks close before outer blocks
    // at the same position, and outer blocks open before inner blocks.
    let apply_block_events = |arrow_row: &mut usize,
                              pos: usize,
                              top_rows: &mut [usize],
                              bottom_rows: &mut [usize],
                              dividers: &mut [Vec<usize>]| {
        // Bottom borders first (innermost first = forward order in
        // diag.blocks since LIFO close order means innermost has
        // lower idx).
        for (i, b) in diag.blocks.iter().enumerate() {
            if pos > 0 && b.end_message + 1 == pos {
                bottom_rows[i] = *arrow_row;
                *arrow_row += 2;
            }
        }
        // Top borders next, outermost first (outer block has higher
        // idx in diag.blocks because it closed later — iterate REV).
        for (i, b) in diag.blocks.iter().enumerate().rev() {
            if b.start_message == pos {
                top_rows[i] = *arrow_row;
                *arrow_row += 2;
            }
        }
        // Branch dividers (continuation rows). Order doesn't matter
        // visually since each belongs to a different block.
        for (i, b) in diag.blocks.iter().enumerate() {
            for (j, branch) in b.branches.iter().enumerate().skip(1) {
                if branch.start_message == pos {
                    dividers[i][j] = *arrow_row;
                    *arrow_row += 2;
                }
            }
        }
    };

    // Helper closure: render any notes whose `after_message` matches
    // `at`, advancing `arrow_row` by each note's height. Uses the
    // pre-computed wrapped text so drawing and height accounting agree.
    let render_notes_at = |canvas: &mut Canvas, arrow_row: &mut usize, at: usize| {
        for (note, wrapped) in diag
            .notes
            .iter()
            .zip(note_wrapped.iter())
            .filter(|(n, _)| n.after_message == at)
        {
            let text_w = max_line_width(wrapped);
            if let Some((l, r)) = note_columns(&note.anchor, &layouts, diag, text_w) {
                draw_note_box(canvas, l, r, *arrow_row, wrapped);
                *arrow_row += wrapped.lines().count().max(1) + 3;
            }
        }
    };

    // Notes positioned BEFORE any message (after_message == 0) land
    // at the top of the body, before the first message label.
    render_notes_at(&mut canvas, &mut arrow_row, 0);

    // Block events at position 0 (any block opening before the first
    // message) land here, before the first message's label row.
    apply_block_events(
        &mut arrow_row,
        0,
        &mut block_top_rows,
        &mut block_bottom_rows,
        &mut branch_divider_rows,
    );

    for (msg_idx, msg) in diag.messages.iter().enumerate() {
        // Block events for this message-index boundary (skip msg_idx == 0
        // because we already applied position 0 events above).
        if msg_idx > 0 {
            apply_block_events(
                &mut arrow_row,
                msg_idx,
                &mut block_top_rows,
                &mut block_bottom_rows,
                &mut branch_divider_rows,
            );
        }

        // Apply any autonumber state changes whose `at_message` index
        // is now reached. Multiple changes at the same index land in
        // source order; the last wins.
        while autonumber_cursor < diag.autonumber_changes.len()
            && diag.autonumber_changes[autonumber_cursor].at_message <= msg_idx
        {
            autonumber = diag.autonumber_changes[autonumber_cursor].state;
            autonumber_cursor += 1;
        }

        // Prefix the label with `[N] ` when autonumber is active.
        // Bumps `next_value` after each numbered message.
        let label_owned;
        let label: &str = match autonumber {
            AutonumberState::On { next_value } => {
                label_owned = if msg.text.is_empty() {
                    format!("[{next_value}]")
                } else {
                    format!("[{next_value}] {}", msg.text)
                };
                autonumber = AutonumberState::On {
                    next_value: next_value + 1,
                };
                &label_owned
            }
            AutonumberState::Off => &msg.text,
        };

        let Some(si) = diag.participant_index(&msg.from) else {
            continue;
        };
        let Some(ti) = diag.participant_index(&msg.to) else {
            continue;
        };

        // Capture the arrow row for this message before advancing.
        message_arrow_rows.push(arrow_row);

        if si == ti {
            draw_self_message(&mut canvas, layouts[si].center, arrow_row, label, msg.style);
            arrow_row += SELF_MSG_ROW_H;
        } else {
            draw_message(
                &mut canvas,
                layouts[si].center,
                layouts[ti].center,
                arrow_row,
                label,
                msg.style,
            );
            arrow_row += EVENT_ROW_H;
        }

        // Render notes positioned AFTER this message (those whose
        // `after_message` index equals this iteration's index + 1
        // — see NoteEvent::after_message docs in src/sequence.rs).
        render_notes_at(&mut canvas, &mut arrow_row, msg_idx + 1);
    }

    // Trailing block-close events: any block whose end_message + 1
    // equals num_messages (i.e., closes after the last message) needs
    // its bottom border drawn here.
    apply_block_events(
        &mut arrow_row,
        num_messages,
        &mut block_top_rows,
        &mut block_bottom_rows,
        &mut branch_divider_rows,
    );

    // 4. Overlay activation bars on participant lifelines. Drawn last so
    //    they sit on top of the dashed lifeline glyph but skip cells
    //    already holding arrow / junction characters from messages.
    //
    //    The range starts at the *label row* of the activating message
    //    (arrow_row - 1) so single-message activations still produce a
    //    visible bar even when the arrow row itself is overwritten by
    //    arrow chars.
    for act in &diag.activations {
        let Some(pi) = diag.participant_index(&act.participant) else {
            continue;
        };
        let cx = layouts[pi].center;
        let arrow_r0 = message_arrow_rows
            .get(act.start_message)
            .copied()
            .unwrap_or(BOX_HEIGHT + 1);
        let r1 = message_arrow_rows
            .get(act.end_message)
            .copied()
            .unwrap_or_else(|| height.saturating_sub(2));
        // Include the label row above the activating message so the bar
        // is at least 2 rows tall (label + arrow), guaranteeing
        // visibility even when start == end.
        let r0 = arrow_r0.saturating_sub(1).max(BOX_HEIGHT);
        let (lo, hi) = if r0 <= r1 { (r0, r1) } else { (r1, r0) };
        for r in lo..=hi {
            for dx in 0..ACTIVATION_BAR_WIDTH {
                let col = cx + dx;
                if col >= canvas.width {
                    break;
                }
                let cell = canvas.grid[r][col];
                if cell == LIFELINE || cell == ' ' {
                    canvas.put(r, col, ACTIVATION_BAR);
                }
            }
        }
    }

    // 5. Overlay block frames. Each block draws a labelled rectangle
    //    spanning the column range of its inner messages, inset by one
    //    cell per nesting level so nested blocks read distinctly. Drawn
    //    last so side rails sit on top of lifelines / activation bars
    //    (still skipping arrow / junction glyphs to read as "behind"
    //    arrows).
    //
    //    Interior bounds are collected first; the shade fill is applied in a
    //    separate pass (step 6) AFTER all borders and labels are committed so
    //    that inner-frame labels are never overwritten by an outer-frame fill.
    let mut frame_interiors: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut label_rows: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (i, b) in diag.blocks.iter().enumerate() {
        // Empty block (no inner messages) — nothing to draw.
        if b.start_message > b.end_message || message_arrow_rows.get(b.start_message).is_none() {
            continue;
        }
        let Some((natural_left, natural_right)) = block_column_range(b, diag, &layouts) else {
            continue;
        };
        // Depth-based horizontal inset so nested rectangles don't draw
        // on the same columns as their enclosing block(s).
        let depth = block_depth(i, &diag.blocks);
        let max_inset = (natural_right - natural_left) / 4;
        let inset = depth.min(max_inset);
        let left = natural_left.saturating_add(inset);
        let right = natural_right.saturating_sub(inset);
        let top = block_top_rows[i];
        let bottom = block_bottom_rows[i];
        if top == 0 || bottom == 0 || top >= bottom {
            continue;
        }
        let kind_label = block_kind_label(b.kind);
        let opener_label = b.branches.first().map(|br| br.label.as_str()).unwrap_or("");
        // Branch dividers (continuations only — first branch has no
        // divider since it shares the top border).
        let branches: Vec<(usize, &str)> = b
            .branches
            .iter()
            .enumerate()
            .skip(1)
            .map(|(j, branch)| (branch_divider_rows[i][j], branch.label.as_str()))
            .filter(|(row, _)| *row != 0)
            .collect();
        // Record every row that carries label text so the fill pass can
        // skip them and avoid overwriting spaces embedded in labels.
        label_rows.insert(top);
        label_rows.insert(bottom);
        for &(dr, _) in &branches {
            label_rows.insert(dr);
        }
        draw_block_frame(
            &mut canvas,
            top,
            bottom,
            left,
            right,
            kind_label,
            opener_label,
            &branches,
        );
        frame_interiors.push((top, bottom, left, right));
    }

    // 6. Interior fill — applied after ALL block frames and labels are
    //    committed so that no outer-frame fill can overwrite an inner-frame
    //    label.  Only plain space cells (`' '`) are replaced; every other
    //    glyph (arrows, lifelines, activation bars, labels, borders) is
    //    already non-space and is left untouched.  Rows that carry frame
    //    labels (top borders and dividers of any frame) are skipped entirely
    //    so embedded spaces within label text stay as spaces.
    for (top, bottom, left, right) in frame_interiors {
        for r in (top + 1)..bottom {
            if label_rows.contains(&r) {
                continue;
            }
            for c in (left + 1)..right {
                if canvas.grid[r][c] == ' ' {
                    canvas.put(r, c, '\u{2591}');
                }
            }
        }
    }

    canvas.into_string()
}

/// Nesting depth for `blocks[idx]` — the number of *other* blocks that
/// strictly contain its message range. Used to inset nested rectangles
/// so they read distinctly from their parents.
fn block_depth(idx: usize, blocks: &[Block]) -> usize {
    let me = &blocks[idx];
    blocks
        .iter()
        .enumerate()
        .filter(|(j, b)| {
            *j != idx
                && b.start_message <= me.start_message
                && b.end_message >= me.end_message
                && (b.start_message < me.start_message || b.end_message > me.end_message)
        })
        .count()
}

/// Compute the column range `(left, right)` spanned by all messages
/// inside `block`. Returns `None` if no message in the block resolves
/// to a known participant.
fn block_column_range(
    block: &Block,
    diag: &SequenceDiagram,
    layouts: &[ParticipantLayout],
) -> Option<(usize, usize)> {
    let mut min_idx: Option<usize> = None;
    let mut max_idx: Option<usize> = None;
    for msg in &diag.messages[block.start_message..=block.end_message] {
        for id in [&msg.from, &msg.to] {
            if let Some(p) = diag.participant_index(id) {
                min_idx = Some(min_idx.map_or(p, |m| m.min(p)));
                max_idx = Some(max_idx.map_or(p, |m| m.max(p)));
            }
        }
    }
    let lo = min_idx?;
    let hi = max_idx?;
    let left = layouts[lo]
        .center
        .saturating_sub(layouts[lo].box_width / 2 + 1);
    let right = layouts[hi].center + layouts[hi].box_width / 2 + 1;
    Some((left, right))
}

/// Human-readable label for the block kind, used as a tag in the
/// frame's top-left corner. Mirrors Mermaid's text labels.
fn block_kind_label(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Loop => "loop",
        BlockKind::Alt => "alt",
        BlockKind::Opt => "opt",
        BlockKind::Par => "par",
        BlockKind::Critical => "critical",
        BlockKind::Break => "break",
    }
}

/// Draw a `[label]` tag at the given position, inset 2 cells from the
/// left edge of the block frame so it sits cleanly off the corner.
/// Returns the column past the tag (caller can chain another tag).
/// No-op when `label` is empty — keeps callers free of guard noise.
fn draw_tag(canvas: &mut Canvas, row: usize, anchor_left: usize, label: &str) -> usize {
    if label.is_empty() {
        return anchor_left + 2;
    }
    let col = anchor_left + 2;
    let tag = format!("[{label}]");
    let width = tag.chars().count();
    canvas.put_str(row, col, &tag);
    col + width
}

/// Draw a labelled rectangular frame for a sequence-diagram block.
///
/// Uses the heavy double-line glyphs (`╔╗╚╝═║`) to differentiate from
/// participant boxes (square `┌┐└┘`) and notes (rounded `╭╮╰╯`).
///
/// **Tag layout (matches Mermaid):** the kind name and the opener
/// branch's condition are rendered as **two separate** `[…]` tags on
/// the top border row, e.g. `╔═[alt]══[cache hit]═══════╗`, mirroring
/// Mermaid's badge-plus-condition style. Branch continuations
/// (`else`/`and`/`option`) carry their condition as a `[…]` tag on
/// the dashed divider row.
///
/// Defensive: the frame paints into space (' '), lifeline (`┆`), and
/// activation-bar (`┃`) cells only — never overwrites a message arrow
/// or label glyph. This means heavily-populated rows may show partial
/// rails, which is the same trade-off the activation overlay accepts.
#[allow(clippy::too_many_arguments)]
fn draw_block_frame(
    canvas: &mut Canvas,
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
    kind: &str,
    opener_label: &str,
    branches: &[(usize, &str)],
) {
    if right <= left || bottom <= top {
        return;
    }

    let paintable = |ch: char| -> bool { ch == ' ' || ch == LIFELINE || ch == ACTIVATION_BAR };

    // Top border with corners.
    if paintable(canvas.grid[top][left]) {
        canvas.put(top, left, '╔');
    }
    for c in (left + 1)..right {
        if paintable(canvas.grid[top][c]) {
            canvas.put(top, c, '═');
        }
    }
    if paintable(canvas.grid[top][right]) {
        canvas.put(top, right, '╗');
    }

    // Two-tag top border: `[kind]` badge then `[opener_label]` condition,
    // separated by `═` characters. Mermaid renders the kind as a small
    // corner badge with the condition floating beside it; this
    // monospace approximation preserves the same semantic split.
    let after_kind = draw_tag(canvas, top, left, kind);
    if !opener_label.is_empty() {
        // Inset the opener label 2 cells past the kind tag so an `═`
        // separator reads between them (e.g. `[alt]══[cache hit]`).
        draw_tag(canvas, top, after_kind, opener_label);
    }

    // Branch dividers (multi-branch blocks only) — drawn BEFORE the
    // side rails so the `╠`/`╣` junction glyphs claim the rail
    // intersection cells; the rails loop below skips divider rows.
    let divider_row_set: std::collections::HashSet<usize> =
        branches.iter().map(|(r, _)| *r).collect();
    for &(divider_row, branch_label) in branches {
        if divider_row <= top || divider_row >= bottom {
            continue;
        }
        // Side-rail intersections always claim ╠ / ╣ (these are
        // junction glyphs that semantically replace the rail).
        canvas.put(divider_row, left, '╠');
        canvas.put(divider_row, right, '╣');
        for c in (left + 1)..right {
            if paintable(canvas.grid[divider_row][c]) {
                canvas.put(divider_row, c, '┄');
            }
        }
        // Continuation label tag — same `draw_tag` helper as the top
        // border keeps the visual style consistent across all label
        // sites in the frame.
        draw_tag(canvas, divider_row, left, branch_label);
    }

    // Side rails on every row in (top, bottom), skipping divider rows
    // (already painted above with ╠/╣).
    for r in (top + 1)..bottom {
        if divider_row_set.contains(&r) {
            continue;
        }
        if paintable(canvas.grid[r][left]) {
            canvas.put(r, left, '║');
        }
        if paintable(canvas.grid[r][right]) {
            canvas.put(r, right, '║');
        }
    }

    // Bottom border with corners.
    if paintable(canvas.grid[bottom][left]) {
        canvas.put(bottom, left, '╚');
    }
    for c in (left + 1)..right {
        if paintable(canvas.grid[bottom][c]) {
            canvas.put(bottom, c, '═');
        }
    }
    if paintable(canvas.grid[bottom][right]) {
        canvas.put(bottom, right, '╝');
    }
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
