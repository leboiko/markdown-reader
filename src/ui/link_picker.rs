use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// One entry in the link picker: the visible link text and the raw `#anchor`
/// string (without the leading `#`). The target line is resolved live from
/// `heading_anchors` at navigation time so it is always current even if block
/// heights have changed since the picker was opened.
#[derive(Debug, Clone)]
pub struct LinkPickerItem {
    pub text: String,
    pub anchor: String,
}

/// State for the link-picker overlay (opened with `f` in the viewer).
#[derive(Debug, Default)]
pub struct LinkPickerState {
    pub cursor: usize,
    pub items: Vec<LinkPickerItem>,
}

impl LinkPickerState {
    pub fn move_up(&mut self) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        self.cursor = (self.cursor + n - 1) % n;
    }

    pub fn move_down(&mut self) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        self.cursor = (self.cursor + 1) % n;
    }
}

/// Render the link-picker overlay centered on the frame.
pub fn draw(f: &mut Frame, app: &mut App) {
    let Some(picker) = &app.link_picker else {
        return;
    };

    let p = &app.palette;
    let cursor = picker.cursor;
    let items = picker.items.clone();

    let n = items.len();
    if n == 0 {
        return;
    }

    let area = f.area();
    let height = (n.min((area.height as usize).saturating_sub(4)) + 2) as u16;
    let width = 72u16.min(area.width.saturating_sub(2));

    let popup_area = centered_rect(width, height, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Links (j/k navigate, Enter jump, Esc dismiss) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.help_bg));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let visible_rows = inner.height as usize;
    let scroll_offset = if cursor < visible_rows {
        0
    } else {
        cursor - visible_rows + 1
    };

    let rows: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(i, item)| {
            let is_cursor = i == cursor;

            let bullet_style = if is_cursor {
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.dim)
            };
            let text_style = if is_cursor {
                Style::default()
                    .fg(p.selection_fg)
                    .bg(p.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.foreground)
            };
            let anchor_style = Style::default().fg(p.link);

            let bullet = if is_cursor { " > " } else { "   " };
            let text_col = format!("{:<36}", item.text);
            let anchor_col = format!("#{}", item.anchor);

            Line::from(vec![
                Span::styled(bullet, bullet_style),
                Span::styled(text_col, text_style),
                Span::styled(anchor_col, anchor_style),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(rows), inner);
}

/// Handle a key event when the link picker is focused.
///
/// Returns `true` when the picker should remain open.
pub fn handle_key(app: &mut App, code: crossterm::event::KeyCode) -> bool {
    match code {
        crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
            if let Some(p) = app.link_picker.as_mut() {
                p.move_down();
            }
            true
        }
        crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
            if let Some(p) = app.link_picker.as_mut() {
                p.move_up();
            }
            true
        }
        crossterm::event::KeyCode::Enter => {
            // Read the anchor name from the picker item, then close the picker.
            // We look up `target_line` live from `heading_anchors` rather than
            // using the cached value in `LinkPickerItem`, because mermaid block
            // heights may have changed since the picker was opened (async render
            // completes between draws), making any pre-cached line stale.
            let anchor = app
                .link_picker
                .as_ref()
                .and_then(|p| p.items.get(p.cursor))
                .map(|item| item.anchor.clone());
            app.link_picker = None;
            if let Some(anchor) = anchor {
                let target_line = app.tabs.active_tab().and_then(|t| {
                    t.view
                        .heading_anchors
                        .iter()
                        .find(|a| a.anchor == anchor)
                        .map(|a| a.line)
                });
                if let Some(line) = target_line {
                    let vh = app.tabs.view_height;
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let max = tab.view.total_lines.saturating_sub(vh / 2);
                        tab.view.scroll_offset = line.saturating_sub(2).min(max);
                    }
                }
            }
            false
        }
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('f') => {
            app.link_picker = None;
            false
        }
        _ => true,
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}
