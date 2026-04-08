pub mod doc_search_bar;
pub mod file_tree;
pub mod help;
pub mod markdown_view;
pub mod search_bar;
pub mod status_bar;

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
        markdown_view::draw(f, app, chunks[0], app.focus == Focus::Viewer || app.focus == Focus::DocSearch);
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
        markdown_view::draw(f, app, main_chunks[1], app.focus == Focus::Viewer || app.focus == Focus::DocSearch);
    }

    if app.search.active {
        search_bar::draw(f, app, chunks[1]);
    }

    status_bar::draw(f, app, chunks[2]);

    // Doc search bar overlays the bottom of the viewer area.
    if app.doc_search.active {
        doc_search_bar::draw(f, app, viewer_area);
    }

    if app.show_help {
        help::draw(f);
    }
}
