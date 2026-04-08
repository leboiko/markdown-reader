use crate::action::Action;
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
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
}

/// State for in-document text search.
#[derive(Debug, Default)]
pub struct DocSearchState {
    /// Whether the search bar is active.
    pub active: bool,
    /// Current query string.
    pub query: String,
    /// Line indices (in rendered text) that contain a match.
    pub match_lines: Vec<u32>,
    /// Index into `match_lines` for the current match.
    pub current_match: usize,
}

/// Top-level application state.
pub struct App {
    /// Set to `false` to break the event loop and exit.
    pub running: bool,
    /// Which panel is currently focused.
    pub focus: Focus,
    /// File-tree widget state.
    pub tree: FileTreeState,
    /// Markdown viewer widget state.
    pub viewer: MarkdownViewState,
    /// Search overlay state.
    pub search: SearchState,
    /// In-document search state.
    pub doc_search: DocSearchState,
    /// Whether the help overlay is visible.
    pub show_help: bool,
    /// Whether the file tree panel is hidden.
    pub tree_hidden: bool,
    /// Width of the file-tree panel as a percentage (10–80).
    pub tree_width_pct: u16,
    /// Root directory being browsed.
    pub root: PathBuf,
    /// Sender injected into components that need to produce actions.
    pub action_tx: Option<tokio::sync::mpsc::UnboundedSender<Action>>,
}

impl App {
    /// Construct a new `App` rooted at `root`, discovering files immediately.
    pub fn new(root: PathBuf) -> Self {
        let entries = FileEntry::discover(&root);
        let mut tree = FileTreeState::default();
        tree.rebuild(entries);

        Self {
            running: true,
            focus: Focus::Tree,
            tree,
            viewer: MarkdownViewState::default(),
            search: SearchState::default(),
            doc_search: DocSearchState::default(),
            show_help: false,
            tree_hidden: false,
            tree_width_pct: 25,
            root,
            action_tx: None,
        }
    }

    /// Run the main event loop until the user quits.
    ///
    /// Draws the UI, then blocks waiting for the next action from any source
    /// (keyboard, watcher, resize). The terminal guard in `main` ensures the
    /// terminal is restored even if this function returns an error.
    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let (mut events, tx) = EventHandler::new();
        self.action_tx = Some(tx.clone());

        // Clone root to avoid conflicting borrows while spawning the watcher.
        let root_clone = self.root.clone();
        let _watcher = crate::fs::watcher::spawn_watcher(&root_clone, tx.clone());

        loop {
            terminal.draw(|f| crate::ui::draw(f, self))?;

            if let Some(action) = events.next().await {
                self.handle_action(action);
            }

            if !self.running {
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
                // Inline the state change — no recursive dispatch needed.
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
                // Rebuild the tree with fresh entries from disk.
                let entries = FileEntry::discover(&self.root);
                self.tree.rebuild(entries);
                // Reload the open file only if one is loaded.
                if self.viewer.current_path.is_some() {
                    self.reload_current_file();
                }
            }
            Action::Resize(_, _) => {}
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Help overlay: any key dismisses it.
        if self.show_help {
            self.show_help = false;
            return;
        }
        // Toggle tree panel visibility from any non-search panel.
        if code == KeyCode::Char('H') && self.focus != Focus::Search {
            self.tree_hidden = !self.tree_hidden;
            if self.tree_hidden && self.focus == Focus::Tree {
                self.focus = Focus::Viewer;
            }
            return;
        }
        // Toggle help from any non-search panel.
        if code == KeyCode::Char('?') && self.focus != Focus::Search {
            self.show_help = true;
            return;
        }
        match self.focus {
            Focus::Search => self.handle_search_key(code, modifiers),
            Focus::Tree => self.handle_tree_key(code, modifiers),
            Focus::Viewer => self.handle_viewer_key(code, modifiers),
            Focus::DocSearch => self.handle_doc_search_key(code, modifiers),
        }
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
                // Inline EnterSearch to avoid a recursive handle_action call.
                self.search.activate();
                self.focus = Focus::Search;
            }
            KeyCode::Char('g') => self.tree.go_first(),
            KeyCode::Char('G') => self.tree.go_last(),
            KeyCode::Char('[') => self.shrink_tree(),
            KeyCode::Char(']') => self.grow_tree(),
            _ => {}
        }
    }

    fn handle_viewer_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
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
            _ => {}
        }
    }

    fn handle_doc_search_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.doc_search.active = false;
                self.focus = Focus::Viewer;
            }
            KeyCode::Enter => {
                // Confirm search and go back to viewer for n/N navigation.
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
                // Inline ExitSearch.
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

        if let Ok(content) = std::fs::read_to_string(&path) {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            self.viewer.load(path, name, content);
            self.focus = Focus::Viewer;
        }
    }

    /// Reload the currently open file from disk, matching by full path rather
    /// than just the file name so that identically-named files in different
    /// directories are not confused.
    fn reload_current_file(&mut self) {
        let Some(current_path) = self.viewer.current_path.clone() else {
            return;
        };
        if let Ok(content) = std::fs::read_to_string(&current_path) {
            let name = current_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            self.viewer.load(current_path, name, content);
        }
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
                                break; // one result per file for now
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
            // Extract the fields we need before moving result into load().
            let path = result.path.clone();
            let name = result.name;
            let result_path = result.path;

            self.viewer.load(path, name, content);
            self.search.active = false;
            self.focus = Focus::Viewer;

            // Sync tree selection to the opened file.
            for (i, item) in self.tree.flat_items.iter().enumerate() {
                if item.path == result_path {
                    self.tree.list_state.select(Some(i));
                    break;
                }
            }
        }
    }

    /// Shrink the tree panel by 5%.
    fn shrink_tree(&mut self) {
        self.tree_width_pct = self.tree_width_pct.saturating_sub(5).max(10);
    }

    /// Grow the tree panel by 5%.
    fn grow_tree(&mut self) {
        self.tree_width_pct = (self.tree_width_pct + 5).min(80);
    }

    /// Search the rendered document lines for the current query.
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

        // Jump to the first match.
        if let Some(&line) = self.doc_search.match_lines.first() {
            self.viewer.scroll_offset = line;
        }
    }

    /// Jump to the next match in the document.
    fn doc_search_next(&mut self) {
        if self.doc_search.match_lines.is_empty() {
            return;
        }
        self.doc_search.current_match =
            (self.doc_search.current_match + 1) % self.doc_search.match_lines.len();
        let line = self.doc_search.match_lines[self.doc_search.current_match];
        self.viewer.scroll_offset = line;
    }

    /// Jump to the previous match in the document.
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
