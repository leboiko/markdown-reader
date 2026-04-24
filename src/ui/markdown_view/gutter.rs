use crate::theme::Palette;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Style,
    text::{Line, Span, Text},
    widgets::Paragraph,
};

/// Render a slice of text with an absolute-line-number gutter.
///
/// `first_line_number` is the 1-based absolute display line of the slice's
/// first row; `total_doc_lines` is used to size the gutter so width is stable
/// across blocks. `scroll_skip` is the number of visual rows to skip from
/// the top of `text` (matches the `scroll((scroll_skip, 0))` applied to the
/// content paragraph).
///
/// `physical_to_logical` controls gutter numbering for Text blocks:
/// - `Some(mapping)` — a pre-wrapped Text block. A source-line number is
///   emitted only when the physical row index `i` maps to a different logical
///   line than row `i-1` (i.e. when `physical_to_logical[i] != physical_to_logical[i-1]`
///   or `i == 0`); continuation rows of the same logical line get a blank gutter
///   entry so the number stays adjacent to its first row.
/// - `None` — a pre-sliced Table block. Every visible row is numbered
///   sequentially from `first_line_number` (tables have no wrapped rows).
#[allow(clippy::too_many_arguments)]
pub fn render_text_with_gutter(
    f: &mut Frame,
    rect: ratatui::layout::Rect,
    text: Text<'static>,
    first_line_number: u32,
    total_doc_lines: u32,
    p: &Palette,
    scroll_skip: u16,
    physical_to_logical: Option<&[u32]>,
) {
    let num_digits = if total_doc_lines == 0 {
        4
    } else {
        (total_doc_lines.ilog10() + 1).max(4)
    };
    let gutter_width = num_digits + 3;

    let chunks = Layout::horizontal([
        Constraint::Length(crate::cast::u16_from_u32(gutter_width)),
        Constraint::Min(0),
    ])
    .split(rect);

    let gutter_style = Style::new().fg(p.gutter);
    let gutter_lines = build_gutter_lines(
        text.lines.len(),
        first_line_number,
        num_digits as usize,
        physical_to_logical,
        gutter_style,
    );

    let mut gutter_para = Paragraph::new(Text::from(gutter_lines));
    if scroll_skip > 0 {
        gutter_para = gutter_para.scroll((scroll_skip, 0));
    }
    f.render_widget(gutter_para, chunks[0]);

    // Text blocks are pre-wrapped — no Paragraph::wrap() needed.
    // Tables are pre-sliced — also no wrap.
    let mut content_para = Paragraph::new(text);
    if scroll_skip > 0 {
        content_para = content_para.scroll((scroll_skip, 0));
    }
    f.render_widget(content_para, chunks[1]);
}

/// Build the per-row gutter strings for a given content row count.
///
/// Extracted from [`render_text_with_gutter`] so the line-numbering
/// logic can be unit-tested without a `Frame`. For Text blocks (`p2l =
/// Some(...)`) emits a number on the first physical row of each logical
/// source line and blanks on continuation rows. For Tables (`p2l =
/// None`) every row is numbered sequentially starting at
/// `first_line_number`.
fn build_gutter_lines(
    row_count: usize,
    first_line_number: u32,
    num_digits: usize,
    physical_to_logical: Option<&[u32]>,
    style: Style,
) -> Vec<Line<'static>> {
    let blank_span = Span::styled(format!("{:>num_digits$} | ", "",), style);
    let mut out: Vec<Line<'static>> = Vec::with_capacity(row_count);

    match physical_to_logical {
        Some(p2l) => {
            // Source-line numbering: emit a number when the logical index
            // changes; blank otherwise. `source_number` advances by 1 each
            // time we emit a number — exactly once per logical line.
            let mut source_number = first_line_number;
            let mut prev_logical: Option<u32> = None;
            for i in 0..row_count {
                let current = p2l.get(i).copied();
                let is_new = match (prev_logical, current) {
                    (None, _) | (Some(_), None) => true,
                    (Some(p), Some(c)) => c != p,
                };
                if is_new {
                    out.push(Line::from(Span::styled(
                        format!("{source_number:>num_digits$} | "),
                        style,
                    )));
                    source_number = source_number.saturating_add(1);
                } else {
                    out.push(Line::from(blank_span.clone()));
                }
                prev_logical = current;
            }
        }
        None => {
            for i in 0..row_count {
                out.push(Line::from(Span::styled(
                    format!(
                        "{:>num_digits$} | ",
                        first_line_number + crate::cast::u32_sat(i)
                    ),
                    style,
                )));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::build_gutter_lines;
    use ratatui::style::Style;

    /// Render `build_gutter_lines` to a plain `Vec<String>` of trimmed cells
    /// so the assertions are easy to read. Trims the ` | ` suffix to keep
    /// the snapshot focused on the line-number formatting.
    fn render(
        row_count: usize,
        first_line: u32,
        digits: usize,
        p2l: Option<&[u32]>,
    ) -> Vec<String> {
        build_gutter_lines(row_count, first_line, digits, p2l, Style::default())
            .into_iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end_matches(" | ")
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// A 3-row wrapped paragraph (one logical line) shows the source number
    /// once and blank padding on the two continuation rows.
    #[test]
    fn text_wrapped_logical_line_numbers_once() {
        let p2l = vec![0u32, 0, 0];
        let out = render(3, 1, 4, Some(&p2l));
        assert_eq!(out, vec!["1", "", ""]);
    }

    /// Mixed-height layout: logical 0 wraps to 2 rows, logical 1 stays 1
    /// row, logical 2 wraps to 3 rows. Numbers appear at the first row of
    /// each new logical line; advances by 1 each time.
    #[test]
    fn text_mixed_heights_advance_per_logical_line() {
        let p2l = vec![0u32, 0, 1, 2, 2, 2];
        let out = render(6, 1, 4, Some(&p2l));
        assert_eq!(out, vec!["1", "", "2", "3", "", ""]);
    }

    /// Numbering starts from `first_line_number`, not from 1.
    #[test]
    fn text_starts_from_offset() {
        let p2l = vec![0u32, 1];
        let out = render(2, 105, 4, Some(&p2l));
        assert_eq!(out, vec!["105", "106"]);
    }

    /// Tables (`p2l = None`) number every row sequentially regardless of
    /// content.
    #[test]
    fn table_rows_numbered_sequentially() {
        let out = render(4, 10, 4, None);
        assert_eq!(out, vec!["10", "11", "12", "13"]);
    }

    /// The blank span is exactly `num_digits` spaces — keeps gutter width
    /// stable across rows.
    #[test]
    fn blank_padding_matches_digit_width() {
        let p2l = vec![0u32, 0];
        let lines = build_gutter_lines(2, 1, 4, Some(&p2l), Style::default());
        let blank: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(blank, "     | ", "4 spaces + ` | ` = 7 cells");
    }
}
