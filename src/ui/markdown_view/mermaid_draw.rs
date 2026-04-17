use super::highlight::apply_block_highlight;
use super::state::VisualRange;
use crate::app::App;
use crate::theme::Palette;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// All parameters needed to draw a single mermaid block.
///
/// Bundles the per-block rendering state and cursor context into one struct so
/// [`draw_mermaid_block`] stays within clippy's 7-argument limit.
pub struct MermaidDrawParams<'a> {
    /// Whether the image is fully visible in the viewport.
    pub fully_visible: bool,
    /// Opaque block identifier used to look up the cache entry.
    pub id: crate::markdown::MermaidBlockId,
    /// Raw mermaid source, displayed when the image is not available.
    pub source: &'a str,
    /// Whether the viewer panel currently has keyboard focus.
    pub focused: bool,
    /// Absolute logical-line index of the cursor.
    pub cursor_line: u32,
    /// Inclusive start of the block in absolute logical lines.
    pub block_start: u32,
    /// Exclusive end of the block in absolute logical lines.
    pub block_end: u32,
    /// Active visual-line selection, or `None` in normal mode.
    pub visual_mode: Option<VisualRange>,
}

/// Draw a mermaid block at the given rect, looking up the cache entry.
///
/// When `params.fully_visible` is false (the block is partially scrolled on-
/// or off-screen), skip image rendering and show a placeholder; otherwise the
/// image widget would re-fit to the shrinking rect and visibly jitter.
pub fn draw_mermaid_block(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    p: &Palette,
    params: &MermaidDrawParams,
) {
    use crate::mermaid::MermaidEntry;

    let entry = app.mermaid_cache.get_mut(params.id);

    // Helper: true when the cursor is inside this block and the viewer is focused.
    let cursor_in_block = params.focused
        && params.cursor_line >= params.block_start
        && params.cursor_line < params.block_end;

    match entry {
        None => {
            render_mermaid_placeholder(f, rect, "mermaid diagram", p);
        }
        Some(MermaidEntry::Pending) => {
            render_mermaid_placeholder(f, rect, "rendering\u{2026}", p);
        }
        Some(MermaidEntry::Ready { protocol, .. }) => {
            if params.fully_visible {
                use ratatui_image::{Resize, StatefulImage};
                f.render_widget(
                    Block::default().style(Style::default().bg(p.background)),
                    rect,
                );
                // Render background bars BEFORE the image so they sit underneath.
                // In visual mode draw a bar for every selected row; in normal mode
                // draw one bar for the cursor row.  The image overwrites most of
                // each bar, leaving only a thin coloured strip around the padding.
                let highlighted_rows: Vec<u32> = match params.visual_mode {
                    Some(range) => (0..params.block_end.saturating_sub(params.block_start))
                        .filter(|&offset| range.contains(params.block_start + offset))
                        .collect(),
                    None if cursor_in_block => {
                        vec![params.cursor_line - params.block_start]
                    }
                    None => vec![],
                };
                for row_offset in highlighted_rows {
                    let row_offset = crate::cast::u16_from_u32(row_offset);
                    if row_offset < rect.height {
                        let bar_rect = Rect {
                            x: rect.x,
                            y: rect.y + row_offset,
                            width: rect.width,
                            height: 1,
                        };
                        f.render_widget(
                            Block::default().style(Style::default().bg(p.selection_bg)),
                            bar_rect,
                        );
                    }
                }
                let padded = padded_rect(rect, 4, 1);
                let image = StatefulImage::new().resize(Resize::Fit(None));
                f.render_stateful_widget(image, padded, protocol.as_mut());
            } else {
                render_mermaid_placeholder(f, rect, "scroll to view diagram", p);
            }
        }
        Some(MermaidEntry::Failed(msg)) => {
            let footer = format!("[mermaid \u{2014} {}]", truncate(msg.as_str(), 60));
            let mut text = render_mermaid_source_text(params.source, &footer, p);
            // Apply cursor/selection highlight to the source-text fallback.
            if params.focused {
                apply_block_highlight(
                    &mut text.lines,
                    params.visual_mode,
                    params.cursor_line,
                    params.block_start,
                    params.block_end,
                    0,
                    p.selection_bg,
                );
            }
            render_mermaid_source_styled(f, rect, text, p);
        }
        Some(MermaidEntry::SourceOnly(reason)) => {
            let footer = format!("[mermaid \u{2014} {reason}]");
            let mut text = render_mermaid_source_text(params.source, &footer, p);
            // Apply cursor/selection highlight to the source-text fallback.
            if params.focused {
                apply_block_highlight(
                    &mut text.lines,
                    params.visual_mode,
                    params.cursor_line,
                    params.block_start,
                    params.block_end,
                    0,
                    p.selection_bg,
                );
            }
            render_mermaid_source_styled(f, rect, text, p);
        }
    }
}

/// Shrink `rect` by `h` cells on the left/right and `v` cells on the top/bottom.
/// If the rect is smaller than the total padding, returns it unchanged.
pub fn padded_rect(rect: Rect, h: u16, v: u16) -> Rect {
    if rect.width <= h * 2 || rect.height <= v * 2 {
        return rect;
    }
    Rect {
        x: rect.x + h,
        y: rect.y + v,
        width: rect.width - h * 2,
        height: rect.height - v * 2,
    }
}

/// Render a placeholder box with a centered status message.
pub fn render_mermaid_placeholder(f: &mut Frame, rect: Rect, msg: &str, p: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if inner.height > 0 {
        let line = Line::from(Span::styled(msg.to_string(), p.dim_style()));
        let para = Paragraph::new(Text::from(vec![line])).alignment(Alignment::Center);
        // Center vertically.
        let y_offset = inner.height / 2;
        let target = Rect {
            y: inner.y + y_offset,
            height: 1,
            ..inner
        };
        f.render_widget(para, target);
    }
}

/// Build the styled `Text` for a mermaid source-fallback display.
///
/// Separating text construction from rendering lets callers mutate the lines
/// (e.g., apply cursor highlight) before committing to the frame buffer.
pub fn render_mermaid_source_text(source: &str, footer: &str, p: &Palette) -> Text<'static> {
    let code_style = Style::default().fg(p.code_fg).bg(p.code_bg);
    let dim_style = p.dim_style();

    let mut lines: Vec<Line<'static>> = source
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), code_style)))
        .collect();
    lines.push(Line::from(Span::styled(footer.to_string(), dim_style)));
    Text::from(lines)
}

/// Render a pre-built mermaid source `Text` with a border block.
pub fn render_mermaid_source_styled(f: &mut Frame, rect: Rect, text: Text<'static>, p: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    let para = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// Truncate `s` to at most `max` bytes, returning the valid UTF-8 prefix.
pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
