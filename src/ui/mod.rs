pub mod config_popup;
pub mod copy_menu;
pub mod doc_search_bar;
pub mod file_tree;
pub mod goto_line_bar;
pub mod help;
pub mod markdown_view;
pub mod search_bar;
pub mod status_bar;
pub mod tab_bar;
pub mod tab_picker;
pub mod table_modal;
pub mod table_render;
pub mod tabs;

use crate::app::{App, Focus};
use crate::config::TreePosition;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::Block,
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Paint every cell with the theme background before any widget renders.
    // Without this, cells not covered by a widget keep the terminal's default
    // background, making light themes unreadable on dark terminals.
    f.render_widget(
        Block::default().style(Style::default().bg(app.palette.background)),
        area,
    );

    // Clear stale rects at the start of each draw so hit-tests against the
    // previous frame's layout never fire.
    app.tree_area_rect = None;
    app.viewer_area_rect = None;
    app.tab_bar_rects.clear();
    app.tab_picker_rects.clear();

    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if app.search.active { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(area);

    let has_tabs = !app.tabs.is_empty();
    let tab_bar_height: u16 = if has_tabs { 1 } else { 0 };

    let viewer_area;

    if app.tree_hidden {
        // No tree panel — tab bar spans the full content width.
        let content_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(tab_bar_height), Constraint::Min(1)])
            .split(outer_chunks[0]);

        if has_tabs {
            tab_bar::draw(f, app, content_chunks[0]);
        }

        viewer_area = content_chunks[1];
        app.viewer_area_rect = Some(viewer_area);

        markdown_view::draw(f, app, viewer_area, is_viewer_focused(app.focus));
    } else {
        let (first_pct, second_pct) = match app.tree_position {
            TreePosition::Left => (app.tree_width_pct, 100 - app.tree_width_pct),
            TreePosition::Right => (100 - app.tree_width_pct, app.tree_width_pct),
        };

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(first_pct),
                Constraint::Percentage(second_pct),
            ])
            .split(outer_chunks[0]);

        let (tree_idx, viewer_idx) = match app.tree_position {
            TreePosition::Left => (0, 1),
            TreePosition::Right => (1, 0),
        };

        let tree_area = main_chunks[tree_idx];
        let viewer_col = main_chunks[viewer_idx];

        file_tree::draw(f, app, tree_area, app.focus == Focus::Tree);
        app.tree_area_rect = Some(tree_area);

        // Tab bar sits above the viewer within the viewer column.
        let viewer_col_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(tab_bar_height), Constraint::Min(1)])
            .split(viewer_col);

        if has_tabs {
            tab_bar::draw(f, app, viewer_col_chunks[0]);
        }

        viewer_area = viewer_col_chunks[1];
        app.viewer_area_rect = Some(viewer_area);

        markdown_view::draw(f, app, viewer_area, is_viewer_focused(app.focus));
    }

    if app.search.active {
        search_bar::draw(f, app, outer_chunks[1]);
    }

    status_bar::draw(f, app, outer_chunks[2]);

    let doc_search_active = app.doc_search().map(|ds| ds.active).unwrap_or(false);
    if doc_search_active {
        doc_search_bar::draw(f, app, viewer_area);
    }

    if app.goto_line.active {
        goto_line_bar::draw(f, app, viewer_area);
    }

    if app.show_help {
        help::draw(f, app);
    }

    if app.tab_picker.is_some() {
        tab_picker::draw(f, app);
    }

    if app.table_modal.is_some() {
        table_modal::draw(f, app);
    }

    if let Some(popup_state) = &app.config_popup {
        let popup_state = popup_state.clone();
        config_popup::render_config_popup(
            f,
            &popup_state,
            app.theme,
            app.show_line_numbers,
            app.tree_position,
            &app.palette,
        );
    }

    if let Some(state) = &app.copy_menu {
        let state = state.clone();
        copy_menu::draw(f, &state, &app.palette);
    }
}

/// Returns `true` when the viewer panel should render as focused.
fn is_viewer_focused(focus: Focus) -> bool {
    matches!(
        focus,
        Focus::Viewer
            | Focus::DocSearch
            | Focus::Config
            | Focus::GotoLine
            | Focus::TabPicker
            | Focus::TableModal
            | Focus::CopyMenu
    )
}
