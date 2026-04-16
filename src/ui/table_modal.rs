use crate::app::{App, TableModalState};
use crate::markdown::CellSpans;
use crate::theme::Palette;
use pulldown_cmark::Alignment;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Render the table modal overlay.
///
/// Caches `popup` into `app.table_modal_rect` each frame so the mouse handler
/// can do hit-testing without re-computing the layout.
pub fn draw(f: &mut Frame, app: &mut App) {
    let state = match &app.table_modal {
        Some(s) => s,
        None => return,
    };
    let p = &app.palette;

    let area = f.area();
    let popup = centered_pct(90, 90, area);
    // Cache for mouse hit-testing (see `handle_table_modal_mouse`).
    app.table_modal_rect = Some(popup);
    f.render_widget(Clear, popup);

    let num_cols = state.natural_widths.len();

    let title = format!(
        " Table  {} col{}  h/l col  H/L \u{00bd}pg  q/Esc close ",
        num_cols,
        if num_cols == 1 { "" } else { "s" },
    );

    // Use the viewer background rather than `help_bg` so the grid border
    // colour (which is tuned for contrast against the main background) stays
    // legible.  The focused-border colour around the modal still signals
    // "this is a modal" visually.
    let block = Block::default()
        .title(title)
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.background));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve 1 line for the footer.
    let content_height = inner.height.saturating_sub(1) as usize;

    // Render the table at natural widths with span-aware word-wrap.
    let rendered = render_modal_table(state, p);

    // Apply v_scroll: skip rows of rendered content.
    let v_scroll = state.v_scroll as usize;
    let visible_lines: Vec<&Line<'static>> = rendered
        .lines
        .iter()
        .skip(v_scroll)
        .take(content_height)
        .collect();

    // Apply h_scroll: slice each line at pixel offset h_scroll.
    let h_scroll = state.h_scroll as usize;
    let visible_width = inner.width as usize;

    let sliced_lines: Vec<Line<'static>> = visible_lines
        .iter()
        .map(|line| slice_line_at(line, h_scroll, visible_width))
        .collect();

    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height as u16,
    };

    f.render_widget(Paragraph::new(Text::from(sliced_lines)), content_rect);

    // Footer with scroll info.
    let total_rendered = rendered.lines.len();
    let footer_text = format!(
        " row {}/{} \u{2502} col {}/{} \u{2502} j/k scroll  d/u \u{00bd}pg  g/G top/bot  0/$ h-pan  h/l col ",
        v_scroll.saturating_add(1).min(total_rendered),
        total_rendered,
        h_scroll,
        state.natural_widths.iter().sum::<usize>(),
    );
    let footer_rect = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer_text, p.dim_style()))),
        footer_rect,
    );
}

/// Render the table at natural widths with word-wrap, returning a `Text`
/// whose lines can be sliced for the modal viewport.
///
/// Each cell is wrapped onto multiple lines when its text exceeds the column
/// width. The row's visual height equals the maximum line count among its cells.
fn render_modal_table(state: &TableModalState, p: &Palette) -> Text<'static> {
    let border_style = Style::default().fg(p.table_border);
    let header_style = Style::default()
        .fg(p.table_header)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(p.foreground);

    let col_widths = &state.natural_widths;
    let num_cols = col_widths.len();

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(modal_border_line(
        '┌',
        '─',
        '┬',
        '┐',
        col_widths,
        border_style,
    ));
    emit_wrapped_row(
        &state.headers,
        col_widths,
        &state.alignments,
        border_style,
        header_style,
        num_cols,
        &mut lines,
    );
    lines.push(modal_border_line(
        '├',
        '─',
        '┼',
        '┤',
        col_widths,
        border_style,
    ));

    for row in &state.rows {
        emit_wrapped_row(
            row,
            col_widths,
            &state.alignments,
            border_style,
            cell_style,
            num_cols,
            &mut lines,
        );
    }

    lines.push(modal_border_line(
        '└',
        '─',
        '┴',
        '┘',
        col_widths,
        border_style,
    ));
    Text::from(lines)
}

/// Wrap all cells in a row and emit one `Line` per visual sub-row.
fn emit_wrapped_row(
    cells: &[CellSpans],
    col_widths: &[usize],
    alignments: &[Alignment],
    border_style: Style,
    cell_style: Style,
    num_cols: usize,
    out: &mut Vec<Line<'static>>,
) {
    let wrapped: Vec<Vec<CellSpans>> = (0..num_cols)
        .map(|i| {
            let spans = cells.get(i).map(|s| s.as_slice()).unwrap_or(&[]);
            let w = col_widths.get(i).copied().unwrap_or(1).max(1);
            wrap_cell_spans(spans, w)
        })
        .collect();

    let row_height = wrapped.iter().map(|c| c.len()).max().unwrap_or(1);

    for sub in 0..row_height {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(num_cols * 3 + 1);
        spans.push(Span::styled("│".to_string(), border_style));
        for (i, &w) in col_widths.iter().enumerate().take(num_cols) {
            let alignment = alignments.get(i).copied().unwrap_or(Alignment::None);
            let cell_line = wrapped[i].get(sub).map(|s| s.as_slice()).unwrap_or(&[]);
            let cell_width: usize = cell_line
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            let padding = w.saturating_sub(cell_width);

            // Emit leading space + alignment padding + cell spans + trailing space + border.
            match alignment {
                Alignment::Right => {
                    let pad_str = format!(" {}", " ".repeat(padding));
                    spans.push(Span::styled(pad_str, cell_style));
                    spans.extend(cell_line.iter().cloned());
                    spans.push(Span::styled(" │".to_string(), border_style));
                }
                Alignment::Center => {
                    let left = padding / 2;
                    let right = padding - left;
                    let pad_str = format!(" {}", " ".repeat(left));
                    spans.push(Span::styled(pad_str, cell_style));
                    spans.extend(cell_line.iter().cloned());
                    let trail = format!("{} │", " ".repeat(right));
                    spans.push(Span::styled(trail, border_style));
                }
                Alignment::Left | Alignment::None => {
                    spans.push(Span::styled(" ".to_string(), cell_style));
                    spans.extend(cell_line.iter().cloned());
                    let trail = format!("{} │", " ".repeat(padding));
                    spans.push(Span::styled(trail, border_style));
                }
            }
        }
        out.push(Line::from(spans));
    }
}

/// A styled grapheme: a single char-sequence (may be multi-byte) with a style.
struct StyledChar {
    ch: char,
    width: usize,
    style: Style,
}

/// Greedy span-aware word-wrap of `cell` to fit within `width` display columns.
///
/// The algorithm:
/// 1. Flatten the span list to a sequence of `StyledChar` values, preserving
///    per-char style.
/// 2. Split into whitespace-separated "words" (each word is a slice of
///    `StyledChar`). Hard newlines in the source (`\n`) force a line break.
/// 3. Greedily pack words onto lines; when a word doesn't fit, start a new line.
/// 4. Words wider than `width` are hard-split at char boundaries.
/// 5. At emit time, adjacent same-style chars are merged into a single `Span`.
///
/// Always returns at least one element (possibly a single empty `Vec`).
pub fn wrap_cell_spans(cell: &[Span<'static>], width: usize) -> Vec<CellSpans> {
    if width == 0 {
        return vec![vec![]];
    }

    // Flatten spans to styled chars.
    let styled: Vec<StyledChar> = cell
        .iter()
        .flat_map(|span| {
            span.content.chars().map(move |ch| StyledChar {
                ch,
                width: UnicodeWidthChar::width(ch).unwrap_or(0),
                style: span.style,
            })
        })
        .collect();

    if styled.is_empty() {
        return vec![vec![]];
    }

    // Split into hard lines at '\n', then into words by whitespace.
    // Each word is a Vec<&StyledChar>.
    let mut result: Vec<CellSpans> = Vec::new();

    // Iterate hard lines.
    let mut line_start = 0;
    while line_start <= styled.len() {
        // Find the next '\n' or end of input.
        let line_end = styled[line_start..]
            .iter()
            .position(|sc| sc.ch == '\n')
            .map(|p| line_start + p)
            .unwrap_or(styled.len());

        let hard_line = &styled[line_start..line_end];
        emit_wrapped_hard_line(hard_line, width, &mut result);

        if line_end >= styled.len() {
            break;
        }
        line_start = line_end + 1;
    }

    if result.is_empty() {
        result.push(vec![]);
    }
    result
}

/// Wrap a single hard line (no embedded newlines) and push output lines to `out`.
///
/// Words are whitespace-separated runs of styled chars. Words that fit on the
/// current line are appended with a space separator. Words wider than `width`
/// are hard-split at char boundaries.
fn emit_wrapped_hard_line(chars: &[StyledChar], width: usize, out: &mut Vec<CellSpans>) {
    // Collect whitespace-separated words as index ranges into `chars`.
    let mut words: Vec<&[StyledChar]> = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, sc) in chars.iter().enumerate() {
        if sc.ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                words.push(&chars[start..i]);
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        words.push(&chars[start..]);
    }

    if words.is_empty() {
        out.push(vec![]);
        return;
    }

    // Each output line is built as an owned Vec of (char, style).
    // Using owned tuples avoids borrow complexity with the mutable accumulator.
    let mut line_buf: Vec<(char, Style)> = Vec::new();
    let mut line_w = 0usize;

    let flush = |buf: &mut Vec<(char, Style)>, out: &mut Vec<CellSpans>| {
        out.push(merge_char_style_pairs(buf));
        buf.clear();
    };

    for word in &words {
        let word_w: usize = word.iter().map(|sc| sc.width).sum();

        if word_w <= width {
            if line_w > 0 && line_w + 1 + word_w > width {
                flush(&mut line_buf, out);
                line_w = 0;
            }
            if line_w > 0 {
                let space_style = word.first().map(|sc| sc.style).unwrap_or_default();
                line_buf.push((' ', space_style));
                line_w += 1;
            }
            for sc in *word {
                line_buf.push((sc.ch, sc.style));
            }
            line_w += word_w;
        } else {
            // Word wider than column — hard-split at char boundaries.
            if line_w > 0 {
                flush(&mut line_buf, out);
            }
            let mut chunk_w = 0usize;
            for sc in *word {
                if chunk_w + sc.width > width {
                    flush(&mut line_buf, out);
                    chunk_w = 0;
                }
                line_buf.push((sc.ch, sc.style));
                chunk_w += sc.width;
            }
            line_w = chunk_w;
        }
    }

    if !line_buf.is_empty() {
        out.push(merge_char_style_pairs(&line_buf));
    }
}

/// Merge a sequence of `(char, Style)` pairs into a `CellSpans`, grouping
/// adjacent same-style chars into single `Span` values.
fn merge_char_style_pairs(pairs: &[(char, Style)]) -> CellSpans {
    let mut spans: CellSpans = Vec::new();
    for &(ch, style) in pairs {
        if let Some(last) = spans.last_mut()
            && last.style == style
        {
            let mut s = last.content.to_string();
            s.push(ch);
            *last = Span::styled(s, style);
        } else {
            spans.push(Span::styled(ch.to_string(), style));
        }
    }
    spans
}

fn modal_border_line(
    left: char,
    fill: char,
    mid: char,
    right: char,
    col_widths: &[usize],
    style: Style,
) -> Line<'static> {
    let mut s = String::new();
    s.push(left);
    for (i, &w) in col_widths.iter().enumerate() {
        for _ in 0..(w + 2) {
            s.push(fill);
        }
        if i + 1 < col_widths.len() {
            s.push(mid);
        }
    }
    s.push(right);
    Line::from(Span::styled(s, style))
}

/// Extract the visible horizontal slice `[h_scroll, h_scroll + visible_width)` from a line.
///
/// Handles multi-byte and double-width characters correctly by walking display
/// columns. Partially-visible double-width characters at the left edge are
/// replaced with a space.
pub fn slice_line_at(line: &Line<'static>, h_scroll: usize, visible_width: usize) -> Line<'static> {
    if visible_width == 0 {
        return Line::from("");
    }

    // Flatten the line to a single string for column-based slicing.
    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let sliced = slice_row(&full, h_scroll, visible_width);

    // Preserve the style of the first span that has content (good enough for modal).
    let style = line.spans.first().map(|s| s.style).unwrap_or_default();
    Line::from(Span::styled(sliced, style))
}

/// Extract a visible horizontal slice from a rendered row string.
///
/// `h_scroll` is the number of display columns to skip from the left.
/// `visible_width` is the maximum number of display columns to return.
/// Double-width characters that straddle the left edge are replaced with a space.
pub fn slice_row(row: &str, h_scroll: usize, visible_width: usize) -> String {
    if visible_width == 0 {
        return String::new();
    }

    // Walk through the row accumulating display columns.
    let mut col = 0usize;
    let mut result = String::with_capacity(visible_width + 4);

    for ch in row.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

        if col + ch_w <= h_scroll {
            col += ch_w;
            continue;
        }

        if col < h_scroll {
            // Double-width char straddles the left edge — emit a replacement space.
            result.push(' ');
            col = h_scroll + 1;
            continue;
        }

        let used = result
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum::<usize>();

        if used + ch_w > visible_width {
            break;
        }

        result.push(ch);
        col += ch_w;
    }

    result
}

fn centered_pct(w_pct: u16, h_pct: u16, area: Rect) -> Rect {
    let w = (area.width * w_pct / 100).max(10);
    let h = (area.height * h_pct / 100).max(5);
    let vertical = Layout::vertical([Constraint::Length(h)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(w)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

/// Compute the maximum visible horizontal extent of the rendered table.
pub fn max_h_scroll(state: &TableModalState, visible_width: u16) -> u16 {
    let total_table_width: usize = state.natural_widths.iter().sum::<usize>()
        + state.natural_widths.len() * 3  // padding + borders per col
        + 1; // right border
    (total_table_width.saturating_sub(visible_width as usize)) as u16
}

/// Return the column-start offsets (in display columns) for each column.
///
/// Each column occupies `width + 3` display columns: one leading space, the
/// cell content, one trailing space, and one border character. This matches
/// the formula used by [`max_h_scroll`].
///
/// The returned slice contains one entry per column. Offset `0` is the leading
/// outer border, so column `i` starts at `sum(widths[0..i]) + i * 3`.
fn col_boundaries(widths: &[usize]) -> Vec<u16> {
    let mut offsets = Vec::with_capacity(widths.len());
    let mut acc: usize = 0;
    for &w in widths {
        offsets.push(acc as u16);
        // Each column occupies: 1 leading space + w content + 1 trailing space
        // + 1 border = w + 3 display columns.
        acc += w + 3;
    }
    offsets
}

/// Snap `h_scroll` to the start of the previous column.
///
/// Finds the largest column-start offset strictly less than `h_scroll` and
/// returns it. If `h_scroll` is already at or before the first boundary (0),
/// returns 0.
///
/// # Examples
///
/// ```
/// // widths [10, 20, 15] → boundaries [0, 13, 36]
/// // From 17 (inside col 1, which starts at 13), previous boundary is 13.
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 17), 13);
/// // From 13 exactly, previous boundary is 0 (start of col 0).
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 13), 0);
/// // From 0 there is no earlier boundary — stays at 0.
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 0), 0);
/// ```
pub fn prev_col_boundary(widths: &[usize], h_scroll: u16) -> u16 {
    // rfind on the sorted boundary list returns the last boundary strictly less
    // than h_scroll, which is the start of the column we are currently inside.
    col_boundaries(widths)
        .into_iter()
        .rfind(|&b| b < h_scroll)
        .unwrap_or(0)
}

/// Snap `h_scroll` to the start of the next column, clamped to `max`.
///
/// Finds the smallest column-start offset strictly greater than `h_scroll` and
/// returns it, clamped to `max`. If there is no such boundary, returns `max`.
///
/// # Examples
///
/// ```
/// // widths [10, 20, 15] → boundaries [0, 13, 36]
/// assert_eq!(next_col_boundary(&[10, 20, 15], 0, 100), 13);
/// assert_eq!(next_col_boundary(&[10, 20, 15], 36, 100), 100);
/// ```
pub fn next_col_boundary(widths: &[usize], h_scroll: u16, max: u16) -> u16 {
    col_boundaries(widths)
        .into_iter()
        // Strictly greater than current position.
        .find(|&b| b > h_scroll)
        .unwrap_or(max)
        .min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn plain(s: &str) -> CellSpans {
        vec![Span::raw(s.to_string())]
    }

    fn styled_span(s: &str, style: Style) -> Span<'static> {
        Span::styled(s.to_string(), style)
    }

    fn spans_text(spans: &CellSpans) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn lines_text(lines: &[CellSpans]) -> Vec<String> {
        lines.iter().map(spans_text).collect()
    }

    // ── wrap_cell_spans tests ────────────────────────────────────────────────

    #[test]
    fn wrap_spans_short_fits_single_line() {
        let cell = plain("hello world");
        let result = wrap_cell_spans(&cell, 20);
        assert_eq!(result.len(), 1);
        assert_eq!(spans_text(&result[0]), "hello world");
    }

    #[test]
    fn wrap_spans_long_wraps_on_word_boundary() {
        let cell = plain("one two three four five");
        let result = wrap_cell_spans(&cell, 10);
        assert!(
            result.len() > 1,
            "should produce multiple lines: {result:?}"
        );
        for line in &result {
            let w: usize = line
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert!(w <= 10, "line too wide: {w}");
        }
        let joined = lines_text(&result).join(" ");
        assert!(joined.contains("one"));
        assert!(joined.contains("five"));
    }

    #[test]
    fn wrap_spans_style_preserved_across_wrap() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let cell: CellSpans = vec![
            styled_span("bold-word ", bold),
            Span::raw("plain continues with more words"),
        ];
        let result = wrap_cell_spans(&cell, 12);
        assert!(result.len() > 1, "should wrap: {result:?}");
        let first_line = &result[0];
        let has_bold = first_line.iter().any(|s| s.style == bold);
        assert!(
            has_bold,
            "first line should contain bold span: {first_line:?}"
        );
    }

    #[test]
    fn wrap_spans_bold_then_plain_splits_in_plain() {
        let bold = Style::default().fg(Color::Red);
        let cell: CellSpans = vec![styled_span("Bold", bold), Span::raw(" plain-text-here")];
        let result = wrap_cell_spans(&cell, 8);
        assert!(
            result.len() > 1,
            "should produce multiple lines: {result:?}"
        );
        let first_text = spans_text(&result[0]);
        assert!(
            first_text.contains("Bold"),
            "first line should have bold: {first_text}"
        );
    }

    #[test]
    fn wrap_spans_word_longer_than_width_hard_splits() {
        let cell = plain("abcdefghij");
        let result = wrap_cell_spans(&cell, 4);
        assert!(result.len() >= 2, "long word must hard-split: {result:?}");
        for line in &result {
            let w: usize = line
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert!(w <= 4, "hard-split chunk too wide: {w}");
        }
        let all_text: String = result
            .iter()
            .flat_map(|l| l.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(all_text, "abcdefghij");
    }

    #[test]
    fn wrap_spans_empty_cell_single_empty_line() {
        let result = wrap_cell_spans(&[], 10);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_empty());
    }

    #[test]
    fn wrap_spans_hard_newline_honored() {
        let cell: CellSpans = vec![Span::raw("line one\nline two".to_string())];
        let result = wrap_cell_spans(&cell, 40);
        assert_eq!(result.len(), 2);
        assert_eq!(spans_text(&result[0]), "line one");
        assert_eq!(spans_text(&result[1]), "line two");
    }

    // ── slice_row tests ──────────────────────────────────────────────────────

    #[test]
    fn slice_row_ascii_mid_column() {
        let row = "abcdefghij";
        assert_eq!(slice_row(row, 3, 4), "defg");
    }

    #[test]
    fn slice_row_start_of_row() {
        let row = "hello world";
        assert_eq!(slice_row(row, 0, 5), "hello");
    }

    #[test]
    fn slice_row_past_end() {
        let row = "short";
        assert_eq!(slice_row(row, 10, 5), "");
    }

    #[test]
    fn slice_row_double_width_straddle() {
        let row = "AB\u{30A2}CD";
        let result = slice_row(row, 3, 5);
        assert!(
            result.starts_with(' '),
            "cut double-width char should be replaced with space: {result:?}"
        );
        assert!(result.contains('C'), "C should be visible: {result:?}");
    }

    #[test]
    fn slice_row_exact_visible_width() {
        let row = "12345678";
        assert_eq!(slice_row(row, 2, 4), "3456");
    }

    // ── column boundary helper tests ─────────────────────────────────────────

    /// widths [10, 20, 15] produce boundaries [0, 13, 36]:
    ///   col 0 starts at 0
    ///   col 1 starts at 10 + 3 = 13
    ///   col 2 starts at 13 + 20 + 3 = 36
    #[test]
    fn prev_col_boundary_jumps_to_start_of_current_column() {
        let widths = [10usize, 20, 15];
        // From 17 (inside col 1, which starts at 13), largest boundary < 17 is 13.
        assert_eq!(prev_col_boundary(&widths, 17), 13);
        // From 13 exactly (on a boundary), largest boundary < 13 is 0 (start of col 0).
        assert_eq!(prev_col_boundary(&widths, 13), 0);
        // From 0 there is no earlier boundary — stays at 0.
        assert_eq!(prev_col_boundary(&widths, 0), 0);
    }

    #[test]
    fn next_col_boundary_jumps_past_current_column() {
        let widths = [10usize, 20, 15];
        // From 0, the next boundary is 13 (start of col 1).
        assert_eq!(next_col_boundary(&widths, 0, 200), 13);
        // From 36 (last boundary), no further boundary → clamp to max.
        assert_eq!(next_col_boundary(&widths, 36, 100), 100);
        // From 13, the next boundary is 36 (start of col 2).
        assert_eq!(next_col_boundary(&widths, 13, 200), 36);
    }

    #[test]
    fn boundary_helpers_handle_empty_widths() {
        // No columns → no movement in either direction.
        assert_eq!(prev_col_boundary(&[], 5), 0);
        assert_eq!(next_col_boundary(&[], 5, 50), 50);
    }
}
