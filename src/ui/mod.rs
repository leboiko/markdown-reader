pub mod config_popup;
pub mod doc_search_bar;
pub mod file_tree;
pub mod goto_line_bar;
pub mod help;
pub mod markdown_view;
pub mod search_bar;
pub mod status_bar;
pub mod tabs;

use crate::app::{App, Focus};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if app.search.active { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(f.area());

    let viewer_area;

    if app.tree_hidden {
        viewer_area = chunks[0];
        markdown_view::draw(
            f,
            app,
            chunks[0],
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
            .split(chunks[0]);

        viewer_area = main_chunks[1];
        file_tree::draw(f, app, main_chunks[0], app.focus == Focus::Tree);
        markdown_view::draw(
            f,
            app,
            main_chunks[1],
            app.focus == Focus::Viewer
                || app.focus == Focus::DocSearch
                || app.focus == Focus::Config
                || app.focus == Focus::GotoLine,
        );
    }

    if app.search.active {
        search_bar::draw(f, app, chunks[1]);
    }

    status_bar::draw(f, app, chunks[2]);

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
