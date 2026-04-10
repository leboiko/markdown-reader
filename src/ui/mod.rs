pub mod config_popup;
pub mod doc_search_bar;
pub mod file_tree;
pub mod goto_line_bar;
pub mod help;
pub mod markdown_view;
pub mod search_bar;
pub mod status_bar;
pub mod tab_bar;
pub mod tabs;

use crate::app::{App, Focus};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if app.search.active { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(area);

    let viewer_area;

    if app.tree_hidden {
        // No tree — tab bar spans the full width of the content area.
        let has_tabs = !app.tabs.is_empty();
        let tab_bar_height = if has_tabs { 1 } else { 0 };

        let content_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(tab_bar_height),
                Constraint::Min(1),
            ])
            .split(outer_chunks[0]);

        if has_tabs {
            tab_bar::draw(f, app, content_chunks[0]);
        }

        viewer_area = content_chunks[1];
        markdown_view::draw(
            f,
            app,
            viewer_area,
            app.focus == Focus::Viewer
                || app.focus == Focus::DocSearch
                || app.focus == Focus::Config
                || app.focus == Focus::GotoLine,
        );
    } else {
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(app.tree_width_pct),
                Constraint::Percentage(100 - app.tree_width_pct),
            ])
            .split(outer_chunks[0]);

        let tree_area = main_chunks[0];
        let viewer_col = main_chunks[1];

        file_tree::draw(f, app, tree_area, app.focus == Focus::Tree);

        // Store the tree area for mouse hit-testing (populated after tree draw).
        app.tree_area_rect = Some(tree_area);

        // Tab bar sits above the viewer within the viewer column.
        let has_tabs = !app.tabs.is_empty();
        let tab_bar_height = if has_tabs { 1 } else { 0 };

        let viewer_col_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(tab_bar_height),
                Constraint::Min(1),
            ])
            .split(viewer_col);

        if has_tabs {
            tab_bar::draw(f, app, viewer_col_chunks[0]);
        }

        viewer_area = viewer_col_chunks[1];
        app.viewer_area_rect = Some(viewer_area);

        markdown_view::draw(
            f,
            app,
            viewer_area,
            app.focus == Focus::Viewer
                || app.focus == Focus::DocSearch
                || app.focus == Focus::Config
                || app.focus == Focus::GotoLine,
        );
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

    if let Some(popup_state) = &app.config_popup {
        let popup_state = popup_state.clone();
        config_popup::render_config_popup(
            f,
            &popup_state,
            app.theme,
            app.show_line_numbers,
            &app.palette,
        );
    }
}

