//! Full-screen mermaid modal renderer.
//!
//! Mirrors `table_modal::draw` in shape: an overlay rect (90% × 90% of the
//! terminal), a titled border, and a 1-line footer with scroll info / key
//! hints. The body dispatches on the cached [`MermaidEntry`] for the active
//! block:
//!
//! - [`MermaidEntry::Ready`] (image): the protocol's `StatefulImage` widget
//!   resizes itself to fit the modal's inner rect via `Resize::Fit(None)`.
//!   Width-overflow gets aspect-preserving downscaling for free; modal
//!   real estate is roughly 5–10× larger than the in-document slot, which
//!   alone fixes most "image is too small to read" cases.
//! - [`MermaidEntry::AsciiDiagram`] (text): the diagram is rendered into a
//!   character grid we slice with `h_scroll` / `v_scroll`, exactly the same
//!   way `table_modal::draw` slices a wide table.
//! - [`MermaidEntry::SourceOnly`] / [`MermaidEntry::Failed`]: show the raw
//!   mermaid source so the user can read what was attempted.
//! - [`MermaidEntry::Pending`] / cache miss: a brief placeholder.
//!
//! The cache is read live every frame, so a background image-render that
//! finishes while the modal is open lights up immediately on the next draw.

use crate::app::App;
use crate::mermaid::MermaidEntry;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use ratatui_image::{Resize, StatefulImage};
use unicode_width::UnicodeWidthStr;

/// Render the mermaid modal overlay. No-op when no modal is active.
///
/// Caches `popup` into `app.mermaid_modal_rect` each frame so the mouse
/// handler can do hit-testing without re-computing the layout.
pub fn draw(f: &mut Frame, app: &mut App) {
    // Copy the small state we need OUT of `app` so we don't hold an
    // immutable borrow while later we need `&mut app.mermaid_cache`.
    let Some((block_id, source, h_scroll, v_scroll)) = app
        .mermaid_modal
        .as_ref()
        .map(|s| (s.block_id, s.source.clone(), s.h_scroll, s.v_scroll))
    else {
        return;
    };
    // Clone the small palette + theme bits we need so the rest of this
    // function can freely take `&mut app` for the protocol render.
    let title_style = app.palette.title_style();
    let border_focused = app.palette.border_focused;
    let background = app.palette.background;
    let foreground = app.palette.foreground;
    let dim_style = app.palette.dim_style();

    let area = f.area();
    let popup = centered_pct(90, 90, area);
    app.mermaid_modal_rect = Some(popup);
    f.render_widget(Clear, popup);

    let title = " Mermaid  j/k scroll  d/u \u{00bd}pg  g/G top/bot  h/l pan  q/Esc close ";

    let block = Block::default()
        .title(title)
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_focused))
        .style(Style::default().bg(background));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve 1 line for the footer.
    let content_height = inner.height.saturating_sub(1);
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height,
    };

    // Live cache lookup — `get_mut` because the image protocol needs `&mut`
    // for stateful rendering. NEVER cache the entry into `MermaidModalState`
    // so background re-renders surface on the next frame.
    let footer_text = match app.mermaid_cache.get_mut(block_id) {
        Some(MermaidEntry::Ready { protocol, .. }) => {
            f.render_stateful_widget(
                StatefulImage::new().resize(Resize::Fit(None)),
                content_rect,
                protocol.as_mut(),
            );
            " image  Esc/Enter close ".to_string()
        }
        Some(MermaidEntry::AsciiDiagram { diagram, .. }) => {
            // Capture the diagram out of the borrow so we can call
            // `draw_text` (which takes `&mut Frame`).
            let diagram = diagram.clone();
            draw_text(f, content_rect, &diagram, h_scroll, v_scroll, foreground);
            text_footer(&diagram, h_scroll, v_scroll)
        }
        Some(MermaidEntry::SourceOnly(reason)) => {
            let reason = reason.clone();
            draw_text(f, content_rect, &source, h_scroll, v_scroll, foreground);
            format!(" source ({reason})  Esc/Enter close ")
        }
        Some(MermaidEntry::Failed(msg)) => {
            let msg = msg.clone();
            draw_text(f, content_rect, &source, h_scroll, v_scroll, foreground);
            format!(" render failed: {msg} \u{2014} Esc/Enter close ")
        }
        Some(MermaidEntry::Pending) | None => {
            let centered = Paragraph::new(Line::from(Span::styled("rendering\u{2026}", dim_style)))
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(centered, content_rect);
            " pending  Esc/Enter close ".to_string()
        }
    };

    let footer_rect = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer_text, dim_style))),
        footer_rect,
    );
}

/// Render `diagram` into `rect` with `h_scroll`/`v_scroll` panning. Matches
/// `table_modal::draw`'s slicing strategy: skip rows for vertical scroll,
/// then trim each visible row to `[h_scroll, h_scroll + width)`.
fn draw_text(
    f: &mut Frame,
    rect: Rect,
    diagram: &str,
    h_scroll: u16,
    v_scroll: u16,
    fg: ratatui::style::Color,
) {
    let v = v_scroll as usize;
    let h = h_scroll as usize;
    let height = rect.height as usize;
    let width = rect.width as usize;

    let visible: Vec<Line<'static>> = diagram
        .lines()
        .skip(v)
        .take(height)
        .map(|line| {
            let sliced = slice_str_at(line, h, width);
            Line::from(Span::styled(sliced, Style::default().fg(fg)))
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(visible)), rect);
}

/// Slice `s` from display-column `start` for at most `width` cells. Handles
/// multi-byte chars and wide chars (CJK / emoji) gracefully — same intent as
/// `table_modal::slice_line_at`, just for a plain &str source.
fn slice_str_at(s: &str, start: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut col = 0usize;
    let mut out = String::with_capacity(width);
    let mut taken = 0usize;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + w <= start {
            col += w;
            continue;
        }
        if taken + w > width {
            break;
        }
        out.push(ch);
        taken += w;
    }
    out
}

/// Build the text-mode footer: row/col counts + key hints.
fn text_footer(diagram: &str, h_scroll: u16, v_scroll: u16) -> String {
    let total_lines = diagram.lines().count();
    let max_width = diagram
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);
    format!(
        " row {}/{} \u{2502} col {}/{} \u{2502} j/k row  d/u \u{00bd}pg  g/G top/bot  h/l pan ",
        (v_scroll as usize).saturating_add(1).min(total_lines),
        total_lines,
        h_scroll as usize,
        max_width,
    )
}

/// Centred popup helper — same shape as `table_modal::centered_pct`.
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

#[cfg(test)]
mod tests {
    use super::slice_str_at;

    #[test]
    fn slice_handles_ascii() {
        assert_eq!(slice_str_at("hello world", 0, 5), "hello");
        assert_eq!(slice_str_at("hello world", 6, 5), "world");
        assert_eq!(slice_str_at("hello world", 11, 5), "");
        assert_eq!(slice_str_at("hello world", 3, 4), "lo w");
    }

    #[test]
    fn slice_zero_width() {
        assert_eq!(slice_str_at("anything", 0, 0), "");
    }

    #[test]
    fn slice_unicode_box_drawing() {
        let s = "┌──┐";
        // start=0, width=4 → entire line.
        assert_eq!(slice_str_at(s, 0, 4), "┌──┐");
        // start=1, width=2 → middle two `─`.
        assert_eq!(slice_str_at(s, 1, 2), "──");
    }
}
