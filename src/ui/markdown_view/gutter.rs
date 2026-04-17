use super::visual_rows::line_visual_rows;
use crate::theme::Palette;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

/// Render a slice of text with an absolute-line-number gutter.
///
/// `first_line_number` is the 1-based absolute display line of the slice's first row;
/// `total_doc_lines` is used to size the gutter so width is stable across blocks.
pub fn render_text_with_gutter(
    f: &mut Frame,
    rect: ratatui::layout::Rect,
    text: Text<'static>,
    first_line_number: u32,
    total_doc_lines: u32,
    p: &Palette,
) {
    let num_digits = if total_doc_lines == 0 {
        4
    } else {
        (total_doc_lines.ilog10() + 1).max(4)
    };
    let gutter_width = num_digits + 3;

    let chunks = Layout::horizontal([Constraint::Length(crate::cast::u16_from_u32(gutter_width)), Constraint::Min(0)])
        .split(rect);

    // The content pane uses `Paragraph::wrap(Wrap { trim: false })`, so a
    // single logical `Line` can occupy multiple visual rows on narrow
    // terminals. The gutter must match that per-row layout: emit the line
    // number on the row where the logical line starts and blank padding on
    // each continuation row, so the number stays visually adjacent to its
    // content.
    let content_width = chunks[1].width;
    let gutter_style = Style::new().fg(p.gutter);
    let mut gutter_lines: Vec<Line<'static>> = Vec::with_capacity(text.lines.len());
    let blank_span = Span::styled(
        format!("{:>width$} | ", "", width = num_digits as usize),
        gutter_style,
    );
    for (i, line) in text.lines.iter().enumerate() {
        gutter_lines.push(Line::from(Span::styled(
            format!(
                "{:>width$} | ",
                first_line_number + crate::cast::u32_sat(i),
                width = num_digits as usize
            ),
            gutter_style,
        )));
        let wraps = line_visual_rows(line, content_width);
        for _ in 1..wraps {
            gutter_lines.push(Line::from(blank_span.clone()));
        }
    }

    f.render_widget(Paragraph::new(Text::from(gutter_lines)), chunks[0]);
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), chunks[1]);
}
