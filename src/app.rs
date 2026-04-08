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
    /// The search overlay.
    Search,
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
        match self.focus {
            Focus::Search => self.handle_search_key(code, modifiers),
            Focus::Tree => self.handle_tree_key(code, modifiers),
            Focus::Viewer => self.handle_viewer_key(code, modifiers),
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

    fn handle_viewer_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
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
                // Inline EnterSearch to avoid a recursive handle_action call.
                self.search.activate();
                self.focus = Focus::Search;
            }
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
}
