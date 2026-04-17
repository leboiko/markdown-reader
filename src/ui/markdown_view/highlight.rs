use super::state::VisualRange;
use crate::theme::Palette;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Decide which lines in a visible block slice need highlighting and apply the
/// background colour to each.
///
/// In **visual mode** every absolute logical line that falls inside the
/// [`VisualRange`] and is also within the visible clip is highlighted. For
/// line-wise mode (`V`) the full line is patched; for char-wise mode (`v`)
/// only the selected column range is patched via [`highlight_columns`].
/// In **normal mode** only the single cursor row is highlighted (full-line).
///
/// # Arguments
///
/// * `lines`       – mutable slice of visible lines already clipped to the viewport.
/// * `visual_mode` – current visual selection, or `None` for normal mode.
/// * `cursor_line` – absolute logical cursor position.
/// * `block_start` – absolute logical line where this block starts.
/// * `block_end`   – exclusive end of the block in absolute logical lines.
/// * `clip_start`  – index within the block of the first visible line (same as
///   the `start` variable used when slicing `visible_text`).
/// * `bg`          – background colour to apply.
pub fn apply_block_highlight(
    lines: &mut [Line<'static>],
    visual_mode: Option<VisualRange>,
    cursor_line: u32,
    block_start: u32,
    block_end: u32,
    clip_start: usize,
    bg: Color,
) {
    match visual_mode {
        Some(range) => {
            // Iterate over absolute logical lines that belong to this block
            // and fall within the visible clip.
            let block_visible_start = block_start + crate::cast::u32_sat(clip_start);
            let block_visible_end =
                block_start + crate::cast::u32_sat(clip_start) + crate::cast::u32_sat(lines.len());
            for abs in block_visible_start..block_visible_end {
                let idx = (abs - block_visible_start) as usize;
                // Compute the display width of this logical line from the current span content.
                let line_width = lines.get(idx).map_or(0, |l| {
                    crate::cast::u16_sat(
                        l.spans
                            .iter()
                            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                            .sum::<usize>(),
                    )
                });
                if let Some((sc, ec)) = range.char_range_on_line(abs, line_width) {
                    if sc == 0 && ec >= line_width {
                        // Full-line highlight — covers line mode and char-mode middle lines.
                        patch_cursor_highlight(lines, idx, bg);
                    } else {
                        // Partial-line highlight — char mode first/last line.
                        if let Some(line) = lines.get(idx) {
                            lines[idx] = highlight_columns(line, sc, ec, bg);
                        }
                    }
                }
            }
        }
        None => {
            // Normal mode: highlight only the cursor row (full line).
            if cursor_line >= block_start && cursor_line < block_end {
                let cursor_relative = (cursor_line - block_start) as usize;
                if cursor_relative >= clip_start {
                    let idx = cursor_relative - clip_start;
                    patch_cursor_highlight(lines, idx, bg);
                }
            }
        }
    }
}

/// Highlight a column range within a single rendered line by splitting spans
/// at the `start_col` and `end_col` boundaries and patching the background of
/// the selected portion.
///
/// Returns a new [`Line`] with the highlight applied. Spans outside the range
/// keep their original style; spans inside get `bg` patched; spans that straddle
/// a boundary are split by walking characters with [`UnicodeWidthChar`], building
/// separate before/inside/after buffers while preserving each span's base style.
///
/// # Arguments
///
/// * `line`      – the rendered line to highlight.
/// * `start_col` – first selected display column (0-based, inclusive).
/// * `end_col`   – one past the last selected display column (exclusive).
/// * `bg`        – background colour for the selected portion.
pub fn highlight_columns(
    line: &Line<'static>,
    start_col: u16,
    end_col: u16,
    bg: Color,
) -> Line<'static> {
    if start_col >= end_col {
        return line.clone();
    }
    let sel_style = Style::default().bg(bg);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut col: u16 = 0;

    for span in &line.spans {
        let span_start_col = col;
        let span_text = span.content.as_ref();
        // Fast path: entire span is outside the selection.
        let span_width = crate::cast::u16_sat(UnicodeWidthStr::width(span_text));
        let span_end_col = col + span_width;

        if span_end_col <= start_col || span_start_col >= end_col {
            // Fully outside: clone unchanged.
            out.push(span.clone());
            col = span_end_col;
            continue;
        }
        if span_start_col >= start_col && span_end_col <= end_col {
            // Fully inside: patch background.
            out.push(Span::styled(
                span.content.clone(),
                span.style.patch(sel_style),
            ));
            col = span_end_col;
            continue;
        }

        // Straddles a boundary — walk characters individually.
        // We build three string buffers: before, inside, after.
        let mut before = String::new();
        let mut inside = String::new();
        let mut after = String::new();
        let mut c_col = span_start_col;
        for ch in span_text.chars() {
            // unicode_width returns 0 for control characters; treat as 1 cell.
            let w = crate::cast::u16_sat(UnicodeWidthChar::width(ch).unwrap_or(1));
            let next = c_col + w;
            if next <= start_col {
                before.push(ch);
            } else if c_col >= end_col {
                after.push(ch);
            } else {
                // Character overlaps the selection boundary or is inside.
                // Put the whole character in whichever region its start falls in.
                if c_col < start_col {
                    // Straddles start boundary: put in `before`.
                    before.push(ch);
                } else {
                    inside.push(ch);
                }
            }
            c_col = next;
        }
        if !before.is_empty() {
            out.push(Span::styled(before, span.style));
        }
        if !inside.is_empty() {
            out.push(Span::styled(inside, span.style.patch(sel_style)));
        }
        if !after.is_empty() {
            out.push(Span::styled(after, span.style));
        }
        col = span_end_col;
    }

    Line::from(out)
}

/// Extract the plain-text content of a rendered line within a display-column
/// range `[start_col, end_col)`.
///
/// Walks spans character-by-character, tracking cumulative display-column
/// position with [`UnicodeWidthChar`]. Characters whose display range falls
/// entirely within `[start_col, end_col)` are collected into the returned
/// [`String`].
///
/// # Arguments
///
/// * `line`      – the rendered line to extract from.
/// * `start_col` – first selected display column (0-based, inclusive).
/// * `end_col`   – one past the last selected display column (exclusive).
pub fn extract_line_text_range(line: &Line<'static>, start_col: u16, end_col: u16) -> String {
    if start_col >= end_col {
        return String::new();
    }
    let mut out = String::new();
    let mut col: u16 = 0;
    for span in &line.spans {
        for ch in span.content.as_ref().chars() {
            let w = crate::cast::u16_sat(UnicodeWidthChar::width(ch).unwrap_or(1));
            let next = col + w;
            if col >= end_col {
                break;
            }
            if next > start_col {
                out.push(ch);
            }
            col = next;
        }
        if col >= end_col {
            break;
        }
    }
    out
}

/// Apply the cursor-highlight background to one row inside a visible slice.
///
/// `lines` is the mutable slice of rendered lines (already clipped to the
/// viewport). `idx` is the 0-based index within that slice that should be
/// highlighted. `bg` is the selection background color.
///
/// Behaviour:
/// - If `idx` is out of bounds, the function is a no-op (no panic).
/// - If the target line has no spans (blank line), a single space span with
///   the background color is injected so the highlight row is still visible.
/// - Otherwise every existing span on that line is patched with `.bg(bg)`.
///
/// All three block types (Text, Table, Mermaid-source) share this helper so
/// the highlight logic lives in exactly one place.
pub fn patch_cursor_highlight(lines: &mut [Line<'static>], idx: usize, bg: Color) {
    let Some(line) = lines.get_mut(idx) else {
        return;
    };
    if line.spans.is_empty() {
        // Blank line — inject a space so the colored row is visible.
        *line = Line::from(Span::styled(" ".to_string(), Style::default().bg(bg)));
    } else {
        for span in &mut line.spans {
            span.style = span.style.patch(Style::default().bg(bg));
        }
    }
}

/// Produce a new `Text` with search matches highlighted.
///
/// `block_start` is the absolute display-line offset of `text`'s first row.
/// It is added to the local line index before comparing against
/// `current_line` (which is absolute), so the "current match" color lands
/// on the right row regardless of which block the match lives in.
pub fn highlight_matches(
    text: &Text<'static>,
    query: &str,
    current_line: Option<u32>,
    block_start: u32,
    p: &Palette,
) -> Text<'static> {
    let query_lower = query.to_lowercase();
    let match_style = Style::default()
        .bg(p.search_match_bg)
        .fg(p.match_fg)
        .add_modifier(Modifier::BOLD);
    let current_style = Style::default()
        .bg(p.current_match_bg)
        .fg(p.match_fg)
        .add_modifier(Modifier::BOLD);

    let lines: Vec<Line<'static>> = text
        .lines
        .iter()
        .enumerate()
        .map(|(line_idx, line)| {
            let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if !line_text.to_lowercase().contains(&query_lower) {
                return line.clone();
            }

            let is_current = current_line == Some(block_start + crate::cast::u32_sat(line_idx));
            let hl_style = if is_current {
                current_style
            } else {
                match_style
            };

            let mut new_spans: Vec<Span<'static>> = Vec::new();
            for span in &line.spans {
                split_and_highlight(
                    &span.content,
                    &query_lower,
                    span.style,
                    hl_style,
                    &mut new_spans,
                );
            }
            Line::from(new_spans)
        })
        .collect();

    Text::from(lines)
}

/// Split `text` on occurrences of `query_lower` (case-folded) and push styled
/// spans into `out`, alternating between `base_style` and `highlight_style`.
fn split_and_highlight(
    text: &str,
    query_lower: &str,
    base_style: Style,
    highlight_style: Style,
    out: &mut Vec<Span<'static>>,
) {
    let text_lower = text.to_lowercase();
    let mut start = 0;

    while let Some(pos) = text_lower[start..].find(query_lower) {
        let abs_pos = start + pos;

        if abs_pos > start {
            out.push(Span::styled(text[start..abs_pos].to_string(), base_style));
        }

        let match_end = abs_pos + query_lower.len();
        out.push(Span::styled(
            text[abs_pos..match_end].to_string(),
            highlight_style,
        ));

        start = match_end;
    }

    if start < text.len() {
        out.push(Span::styled(text[start..].to_string(), base_style));
    }
}
