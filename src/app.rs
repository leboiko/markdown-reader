use crate::action::Action;
use crate::config::Config;
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::state::AppState;
use crate::theme::{Palette, Theme};
use crate::ui::file_tree::FileTreeState;
use crate::ui::markdown_view::MarkdownViewState;
use crate::ui::search_bar::{SearchMode, SearchResult, SearchState};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use std::path::PathBuf;

/// Which panel currently receives keyboard input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    /// The file-tree panel on the left.
    Tree,
    /// The markdown preview panel on the right.
    Viewer,
    /// The file/content search overlay.
    Search,
    /// In-document text search (typing the query).
    DocSearch,
    /// The settings popup.
    Config,
    /// Go-to-line prompt in the viewer.
    GotoLine,
}

/// Transient state for the settings popup.
///
/// The popup exposes a flat list of options across two sections. [`SECTIONS`]
/// describes the layout used by both the handler and the renderer.
#[derive(Debug, Clone, Default)]
pub struct ConfigPopupState {
    /// Currently highlighted row in the flat option list.
    pub cursor: usize,
}

impl ConfigPopupState {
    /// Ordered sections: `(label, option count)`.
    pub const SECTIONS: &'static [(&'static str, usize)] = &[
        ("Theme", Theme::ALL.len()),
        // "Markdown" section has one toggle: show_line_numbers.
        ("Markdown", 1),
    ];

    pub fn total_rows() -> usize {
        Self::SECTIONS.iter().map(|(_, n)| n).sum()
    }

    pub fn move_up(&mut self) {
        let total = Self::total_rows();
        self.cursor = (self.cursor + total - 1) % total;
    }

    pub fn move_down(&mut self) {
        let total = Self::total_rows();
        self.cursor = (self.cursor + 1) % total;
    }
}

/// State for in-document text search.
#[derive(Debug, Default)]
pub struct DocSearchState {
    pub active: bool,
    pub query: String,
    pub match_lines: Vec<u32>,
    pub current_match: usize,
}

/// State for the go-to-line prompt.
#[derive(Debug, Default)]
pub struct GotoLineState {
    pub active: bool,
    pub input: String,
}

/// Top-level application state.
pub struct App {
    /// Set to `false` to break the event loop and exit.
    pub running: bool,
    /// Which panel is currently focused.
    pub focus: Focus,
    /// The focus that was active before the config popup was opened.
    pub pre_config_focus: Focus,
    /// File-tree widget state.
    pub tree: FileTreeState,
    /// Markdown viewer widget state.
    pub viewer: MarkdownViewState,
    /// Search overlay state.
    pub search: SearchState,
    /// In-document search state.
    pub doc_search: DocSearchState,
    /// Go-to-line prompt state.
    pub goto_line: GotoLineState,
    /// Settings popup state; `None` when the popup is closed.
    pub config_popup: Option<ConfigPopupState>,
    /// Whether the help overlay is visible.
    pub show_help: bool,
    /// Whether the file tree panel is hidden.
    pub tree_hidden: bool,
    /// Width of the file-tree panel as a percentage (10–80).
    pub tree_width_pct: u16,
    /// Root directory being browsed.
    pub root: PathBuf,
    /// Active theme.
    pub theme: Theme,
    /// Cached style palette derived from `theme`.
    pub palette: Palette,
    /// Whether to show line numbers in the viewer.
    pub show_line_numbers: bool,
    /// Persisted sessions (loaded once on startup, written on file open and quit).
    pub app_state: AppState,
    /// Sender injected into components that need to produce actions.
    pub action_tx: Option<tokio::sync::mpsc::UnboundedSender<Action>>,
}

impl App {
    /// Construct a new `App` rooted at `root`.
    ///
    /// Loads persisted config and session state, then auto-restores the last
    /// open file if it still exists on disk.
    pub fn new(root: PathBuf) -> Self {
        let config = Config::load();
        let palette = Palette::from_theme(config.theme);
        let app_state = AppState::load();

        let entries = FileEntry::discover(&root);
        let mut tree = FileTreeState::default();
        tree.rebuild(entries);

        let mut app = Self {
            running: true,
            focus: Focus::Tree,
            pre_config_focus: Focus::Tree,
            tree,
            viewer: MarkdownViewState::default(),
            search: SearchState::default(),
            doc_search: DocSearchState::default(),
            goto_line: GotoLineState::default(),
            config_popup: None,
            show_help: false,
            tree_hidden: false,
            tree_width_pct: 25,
            root,
            theme: config.theme,
            palette,
            show_line_numbers: config.show_line_numbers,
            app_state,
            action_tx: None,
        };

        app.restore_session();
        app
    }

    /// If a session exists for the current root and the saved file is still on
    /// disk, load it into the viewer and select it in the tree.
    fn restore_session(&mut self) {
        let session = match self.app_state.sessions.get(&self.root).cloned() {
            Some(s) => s,
            None => return,
        };

        if session.file.as_os_str().is_empty() || !session.file.exists() {
            return;
        }

        // Verify the file is under the current root.
        if !session.file.starts_with(&self.root) {
            return;
        }

        let Ok(content) = std::fs::read_to_string(&session.file) else {
            return;
        };

        let name = session
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        self.viewer
            .load(session.file.clone(), name, content, &self.palette);

        // Clamp scroll to document bounds.
        let max_scroll = self.viewer.total_lines.saturating_sub(1);
        self.viewer.scroll_offset = session.scroll.min(max_scroll);

        // Expand tree directories along the file's path and select it.
        self.expand_and_select(&session.file);
        self.focus = Focus::Viewer;
    }

    /// Expand every ancestor directory of `file` in the tree and select the file.
    fn expand_and_select(&mut self, file: &PathBuf) {
        // Collect ancestors between root and the file.
        let mut to_expand = Vec::new();
        let mut current = file.as_path();
        while let Some(parent) = current.parent() {
            if parent == self.root {
                break;
            }
            if parent.starts_with(&self.root) {
                to_expand.push(parent.to_path_buf());
            }
            current = parent;
        }
        for dir in to_expand {
            self.tree.expanded.insert(dir);
        }
        self.tree.flatten_visible();

        // Select the file in the flat list.
        for (i, item) in self.tree.flat_items.iter().enumerate() {
            if item.path == *file {
                self.tree.list_state.select(Some(i));
                break;
            }
        }
    }

    /// Save the current session (open file + scroll offset) to disk.
    fn save_session(&mut self) {
        let Some(path) = self.viewer.current_path.clone() else {
            return;
        };
        let scroll = self.viewer.scroll_offset;
        let root = self.root.clone();
        self.app_state.update_session(&root, path, scroll);
    }

    /// Persist the current config settings.
    fn persist_config(&self) {
        Config {
            theme: self.theme,
            show_line_numbers: self.show_line_numbers,
        }
        .save();
    }

    /// Run the main event loop until the user quits.
    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let (mut events, tx) = EventHandler::new();
        self.action_tx = Some(tx.clone());

        let root_clone = self.root.clone();
        let _watcher = crate::fs::watcher::spawn_watcher(&root_clone, tx.clone());

        loop {
            terminal.draw(|f| crate::ui::draw(f, self))?;

            if let Some(action) = events.next().await {
                self.handle_action(action);
            }

            if !self.running {
                // Save session state before exiting.
                self.save_session();
                break;
            }
        }

        Ok(())
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::RawKey(key) => self.handle_key(key.code, key.modifiers),
            Action::Quit => self.running = false,
            Action::FocusLeft => self.focus = Focus::Tree,
            Action::FocusRight => self.focus = Focus::Viewer,
            Action::TreeUp => self.tree.move_up(),
            Action::TreeDown => self.tree.move_down(),
            Action::TreeToggle => self.tree.toggle_expand(),
            Action::TreeFirst => self.tree.go_first(),
            Action::TreeLast => self.tree.go_last(),
            Action::TreeSelect => self.open_selected_file(),
            Action::ScrollUp(n) => self.viewer.scroll_up(n),
            Action::ScrollDown(n) => self.viewer.scroll_down(n),
            Action::ScrollHalfPageUp => self.viewer.scroll_half_page_up(),
            Action::ScrollHalfPageDown => self.viewer.scroll_half_page_down(),
            Action::ScrollToTop => self.viewer.scroll_to_top(),
            Action::ScrollToBottom => self.viewer.scroll_to_bottom(),
            Action::EnterSearch => {
                self.search.activate();
                self.focus = Focus::Search;
            }
            Action::ExitSearch => {
                self.search.active = false;
                self.focus = Focus::Tree;
            }
            Action::SearchInput(c) => {
                self.search.query.push(c);
                self.perform_search();
            }
            Action::SearchBackspace => {
                self.search.query.pop();
                self.perform_search();
            }
            Action::SearchNext => self.search.next_result(),
            Action::SearchPrev => self.search.prev_result(),
            Action::SearchToggleMode => {
                self.search.toggle_mode();
                self.perform_search();
            }
            Action::SearchConfirm => self.confirm_search(),
            Action::FilesChanged(_changed) => {
                let entries = FileEntry::discover(&self.root);
                self.tree.rebuild(entries);
                if self.viewer.current_path.is_some() {
                    self.reload_current_file();
                }
            }
            Action::Resize(_, _) => {}
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.show_help {
            self.show_help = false;
            return;
        }

        // Config popup captures all input when open.
        if self.focus == Focus::Config {
            self.handle_config_key(code);
            return;
        }

        if code == KeyCode::Char('H') && self.focus != Focus::Search {
            self.tree_hidden = !self.tree_hidden;
            if self.tree_hidden && self.focus == Focus::Tree {
                self.focus = Focus::Viewer;
            }
            return;
        }
        if code == KeyCode::Char('?') && self.focus != Focus::Search {
            self.show_help = true;
            return;
        }
        match self.focus {
            Focus::Search => self.handle_search_key(code, modifiers),
            Focus::Tree => self.handle_tree_key(code, modifiers),
            Focus::Viewer => self.handle_viewer_key(code, modifiers),
            Focus::DocSearch => self.handle_doc_search_key(code, modifiers),
            Focus::GotoLine => self.handle_goto_line_key(code),
            // Config is handled above; this arm is unreachable but required for exhaustiveness.
            Focus::Config => {}
        }
    }

    fn handle_config_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(popup) = self.config_popup.as_mut() {
                    popup.move_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(popup) = self.config_popup.as_mut() {
                    popup.move_down();
                }
            }
            KeyCode::Enter => {
                if let Some(popup) = self.config_popup.as_ref() {
                    let cursor = popup.cursor;
                    self.apply_config_selection(cursor);
                }
            }
            KeyCode::Esc | KeyCode::Char('c') => {
                self.config_popup = None;
                self.focus = self.pre_config_focus;
            }
            KeyCode::Char('q') => self.running = false,
            _ => {}
        }
    }

    fn apply_config_selection(&mut self, cursor: usize) {
        let theme_count = Theme::ALL.len();

        if cursor < theme_count {
            // Section 0: Theme
            let theme = Theme::ALL[cursor];
            self.theme = theme;
            self.palette = Palette::from_theme(theme);
            // Re-render the open document so heading/code colors update.
            self.rerender_current_doc();
            self.persist_config();
        } else {
            // Section 1: Markdown — only one option at index theme_count.
            self.show_line_numbers = !self.show_line_numbers;
            self.persist_config();
        }
    }

    /// Re-render the current document with the active palette, preserving scroll.
    fn rerender_current_doc(&mut self) {
        let Some(path) = self.viewer.current_path.clone() else {
            return;
        };
        let content = self.viewer.content.clone();
        let name = self.viewer.file_name.clone();
        let scroll = self.viewer.scroll_offset;
        self.viewer.load(path, name, content, &self.palette);
        self.viewer.scroll_offset = scroll.min(self.viewer.total_lines.saturating_sub(1));
    }

    fn handle_tree_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => self.tree.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.tree.move_up(),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(item) = self.tree.selected_item().cloned() {
                    if item.is_dir {
                        self.tree.toggle_expand();
                    } else {
                        self.open_selected_file();
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(item) = self.tree.selected_item().cloned()
                    && item.is_dir
                    && self.tree.expanded.contains(&item.path)
                {
                    self.tree.toggle_expand();
                }
            }
            KeyCode::Tab => self.focus = Focus::Viewer,
            KeyCode::Char('/') => {
                self.search.activate();
                self.focus = Focus::Search;
            }
            KeyCode::Char('g') => self.tree.go_first(),
            KeyCode::Char('G') => self.tree.go_last(),
            KeyCode::Char('[') => self.shrink_tree(),
            KeyCode::Char(']') => self.grow_tree(),
            KeyCode::Char('c') => {
                self.pre_config_focus = Focus::Tree;
                self.config_popup = Some(ConfigPopupState::default());
                self.focus = Focus::Config;
            }
            _ => {}
        }
    }

    fn handle_viewer_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.doc_search.active = false;
                self.doc_search.query.clear();
                self.doc_search.match_lines.clear();
            }
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => self.viewer.scroll_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.viewer.scroll_up(1),
            KeyCode::Char('d') => self.viewer.scroll_half_page_down(),
            KeyCode::Char('u') => self.viewer.scroll_half_page_up(),
            KeyCode::PageDown => self.viewer.scroll_page_down(),
            KeyCode::PageUp => self.viewer.scroll_page_up(),
            KeyCode::Char('g') => self.viewer.scroll_to_top(),
            KeyCode::Char('G') => self.viewer.scroll_to_bottom(),
            KeyCode::Tab => self.focus = Focus::Tree,
            KeyCode::Char('[') => self.shrink_tree(),
            KeyCode::Char(']') => self.grow_tree(),
            KeyCode::Char('/') => {
                self.search.activate();
                self.focus = Focus::Search;
            }
            KeyCode::Char('f') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.doc_search.active = true;
                self.doc_search.query.clear();
                self.doc_search.match_lines.clear();
                self.doc_search.current_match = 0;
                self.focus = Focus::DocSearch;
            }
            KeyCode::Char('n') => self.doc_search_next(),
            KeyCode::Char('N') => self.doc_search_prev(),
            KeyCode::Char('c') => {
                self.pre_config_focus = Focus::Viewer;
                self.config_popup = Some(ConfigPopupState::default());
                self.focus = Focus::Config;
            }
            KeyCode::Char(':') => {
                self.goto_line.active = true;
                self.goto_line.input.clear();
                self.focus = Focus::GotoLine;
            }
            _ => {}
        }
    }

    fn handle_goto_line_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.goto_line.active = false;
                self.goto_line.input.clear();
                self.focus = Focus::Viewer;
            }
            KeyCode::Enter => {
                if let Ok(n) = self.goto_line.input.parse::<u32>()
                    && n > 0
                    && self.viewer.total_lines > 0
                {
                    let max_line = self.viewer.total_lines;
                    let target = n.min(max_line) - 1;
                    self.viewer.scroll_offset = target;
                }
                self.goto_line.active = false;
                self.goto_line.input.clear();
                self.focus = Focus::Viewer;
            }
            KeyCode::Backspace => {
                self.goto_line.input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if self.goto_line.input.len() < 9 {
                    self.goto_line.input.push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_doc_search_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.doc_search.active = false;
                self.doc_search.query.clear();
                self.doc_search.match_lines.clear();
                self.focus = Focus::Viewer;
            }
            KeyCode::Enter => {
                self.focus = Focus::Viewer;
            }
            KeyCode::Backspace => {
                self.doc_search.query.pop();
                self.perform_doc_search();
            }
            KeyCode::Char(c) => {
                self.doc_search.query.push(c);
                self.perform_doc_search();
            }
            KeyCode::Down => self.doc_search_next(),
            KeyCode::Up => self.doc_search_prev(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.search.active = false;
                self.focus = Focus::Tree;
            }
            KeyCode::Enter => self.confirm_search(),
            KeyCode::Backspace => {
                self.search.query.pop();
                self.perform_search();
            }
            KeyCode::Tab => {
                self.search.toggle_mode();
                self.perform_search();
            }
            KeyCode::Down => self.search.next_result(),
            KeyCode::Up => self.search.prev_result(),
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.next_result()
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.prev_result()
            }
            KeyCode::Char(c) => {
                self.search.query.push(c);
                self.perform_search();
            }
            _ => {}
        }
    }

    fn open_selected_file(&mut self) {
        let Some(path) = self.tree.selected_path().map(|p| p.to_path_buf()) else {
            return;
        };

        if path.is_dir() {
            return;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };

        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        self.viewer.load(path.clone(), name, content, &self.palette);
        self.focus = Focus::Viewer;

        // Persist: new file opened, scroll is 0.
        let root = self.root.clone();
        self.app_state.update_session(&root, path, 0);
    }

    fn reload_current_file(&mut self) {
        let Some(current_path) = self.viewer.current_path.clone() else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&current_path) else {
            return;
        };
        let name = current_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let scroll = self.viewer.scroll_offset;
        self.viewer.load(current_path, name, content, &self.palette);
        // Preserve scroll after hot-reload.
        self.viewer.scroll_offset = scroll.min(self.viewer.total_lines.saturating_sub(1));
    }

    fn perform_search(&mut self) {
        self.search.results.clear();
        self.search.selected_index = 0;

        if self.search.query.is_empty() {
            return;
        }

        let query_lower = self.search.query.to_lowercase();

        match self.search.mode {
            SearchMode::FileName => {
                for item in &self.tree.flat_items {
                    if !item.is_dir && item.name.to_lowercase().contains(&query_lower) {
                        self.search.results.push(SearchResult {
                            path: item.path.clone(),
                            name: item.name.clone(),
                            line_number: None,
                            snippet: None,
                        });
                    }
                }
            }
            SearchMode::Content => {
                let paths = FileEntry::flat_paths(&self.tree.entries);
                for path in paths {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        for (i, line) in content.lines().enumerate() {
                            if line.to_lowercase().contains(&query_lower) {
                                let name = path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string();
                                self.search.results.push(SearchResult {
                                    path: path.clone(),
                                    name,
                                    line_number: Some(i + 1),
                                    snippet: Some(line.trim().to_string()),
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    fn confirm_search(&mut self) {
        if let Some(result) = self.search.results.get(self.search.selected_index).cloned()
            && let Ok(content) = std::fs::read_to_string(&result.path)
        {
            let path = result.path.clone();
            let name = result.name;
            let result_path = result.path;

            self.viewer.load(path.clone(), name, content, &self.palette);
            self.search.active = false;
            self.focus = Focus::Viewer;

            for (i, item) in self.tree.flat_items.iter().enumerate() {
                if item.path == result_path {
                    self.tree.list_state.select(Some(i));
                    break;
                }
            }

            let root = self.root.clone();
            self.app_state.update_session(&root, path, 0);
        }
    }

    fn shrink_tree(&mut self) {
        self.tree_width_pct = self.tree_width_pct.saturating_sub(5).max(10);
    }

    fn grow_tree(&mut self) {
        self.tree_width_pct = (self.tree_width_pct + 5).min(80);
    }

    fn perform_doc_search(&mut self) {
        self.doc_search.match_lines.clear();
        self.doc_search.current_match = 0;

        if self.doc_search.query.is_empty() {
            return;
        }

        let query_lower = self.doc_search.query.to_lowercase();

        for (i, line) in self.viewer.rendered.lines.iter().enumerate() {
            let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if line_text.to_lowercase().contains(&query_lower) {
                self.doc_search.match_lines.push(i as u32);
            }
        }

        if let Some(&line) = self.doc_search.match_lines.first() {
            self.viewer.scroll_offset = line;
        }
    }

    fn doc_search_next(&mut self) {
        if self.doc_search.match_lines.is_empty() {
            return;
        }
        self.doc_search.current_match =
            (self.doc_search.current_match + 1) % self.doc_search.match_lines.len();
        let line = self.doc_search.match_lines[self.doc_search.current_match];
        self.viewer.scroll_offset = line;
    }

    fn doc_search_prev(&mut self) {
        if self.doc_search.match_lines.is_empty() {
            return;
        }
        self.doc_search.current_match = if self.doc_search.current_match == 0 {
            self.doc_search.match_lines.len() - 1
        } else {
            self.doc_search.current_match - 1
        };
        let line = self.doc_search.match_lines[self.doc_search.current_match];
        self.viewer.scroll_offset = line;
    }
}
