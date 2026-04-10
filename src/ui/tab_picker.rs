use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// State for the tab picker overlay (opened with `T`).
#[derive(Debug, Default)]
pub struct TabPickerState {
    /// Highlighted row in the picker list (0-based, into the tabs slice).
    pub cursor: usize,
}

impl TabPickerState {
    pub fn move_up(&mut self, tab_count: usize) {
        if tab_count == 0 {
            return;
        }
        self.cursor = (self.cursor + tab_count - 1) % tab_count;
    }

    pub fn move_down(&mut self, tab_count: usize) {
        if tab_count == 0 {
            return;
        }
        self.cursor = (self.cursor + 1) % tab_count;
    }

    /// Clamp the cursor into `[0, max)`. Returns the clamped value.
    pub fn clamp(&mut self, max: usize) {
        if max == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(max - 1);
        }
    }
}

/// Render the tab picker overlay.
///
/// Writes per-row Rects into `app.tab_picker_rects` for mouse hit-testing.
pub fn draw(f: &mut Frame, app: &mut App) {
    app.tab_picker_rects.clear();

    let n = app.tabs.len();
    if n == 0 {
        return;
    }

    let p = &app.palette;
    let picker = match &app.tab_picker {
        Some(s) => s,
        None => return,
    };
    let cursor = picker.cursor;
    let active_idx = app.tabs.active_index();

    let area = f.area();
    let height = (n.min((area.height as usize).saturating_sub(4)) + 2) as u16;
    let width = 64u16.min(area.width.saturating_sub(2));

    let popup_area = centered_rect(width, height, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Tabs (j/k navigate, Enter open, x close, Esc dismiss) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.help_bg));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    // The visible window scrolls so the cursor stays on screen.
    let visible_rows = inner.height as usize;
    let scroll_offset = if cursor < visible_rows {
        0
    } else {
        cursor - visible_rows + 1
    };

    let rows: Vec<Line> = app
        .tabs
        .tabs
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(i, tab)| {
            let num = if i < 9 {
                format!(" {} ", i + 1)
            } else {
                "   ".to_string()
            };

            let file_name = tab.view.file_name.as_str();
            let parent = tab
                .view
                .current_path
                .as_deref()
                .and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();

            let is_active = active_idx == Some(i);
            let is_cursor = i == cursor;

            let num_style = if is_cursor {
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(p.accent_alt).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.dim)
            };

            let name_style = if is_cursor {
                Style::default()
                    .fg(p.selection_fg)
                    .bg(p.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(p.accent_alt).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.foreground)
            };

            let dir_style = Style::default().fg(p.dim);

            // Pad the filename to give some breathing room.
            let name_col = format!("{file_name:<24}");
            let dir_col = format!(" {parent}");

            Line::from(vec![
                Span::styled(num, num_style),
                Span::styled(name_col, name_style),
                Span::styled(dir_col, dir_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(rows);
    f.render_widget(paragraph, inner);

    // Record per-row rects for mouse hit-testing.
    for (slot, (i, tab)) in app
        .tabs
        .tabs
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .enumerate()
    {
        let row_rect = Rect {
            x: inner.x,
            y: inner.y + slot as u16,
            width: inner.width,
            height: 1,
        };
        app.tab_picker_rects.push((tab.id, row_rect));
        let _ = i; // silence unused warning
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

/// Handle a key event when the tab picker is focused.
///
/// Returns `true` when the picker should remain open, `false` when it should close.
pub fn handle_key(app: &mut App, code: crossterm::event::KeyCode) -> bool {
    let n = app.tabs.len();

    match code {
        crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
            if let Some(p) = app.tab_picker.as_mut() {
                p.move_down(n);
            }
            true
        }
        crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
            if let Some(p) = app.tab_picker.as_mut() {
                p.move_up(n);
            }
            true
        }
        crossterm::event::KeyCode::Enter => {
            let cursor = app.tab_picker.as_ref().map(|p| p.cursor).unwrap_or(0);
            if let Some(tab) = app.tabs.tabs.get(cursor) {
                let id = tab.id;
                app.tabs.set_active(id);
            }
            app.tab_picker = None;
            false
        }
        crossterm::event::KeyCode::Char('x') => {
            let cursor = app.tab_picker.as_ref().map(|p| p.cursor).unwrap_or(0);
            if let Some(tab) = app.tabs.tabs.get(cursor) {
                let id = tab.id;
                app.tabs.close(id);
            }
            if app.tabs.is_empty() {
                app.tab_picker = None;
                return false;
            }
            // Clamp cursor after removal.
            let new_n = app.tabs.len();
            if let Some(p) = app.tab_picker.as_mut() {
                p.clamp(new_n);
            }
            true
        }
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('T') => {
            app.tab_picker = None;
            false
        }
        _ => true,
    }
}

