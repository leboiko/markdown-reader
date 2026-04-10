use crate::action::Action;
use crate::config::Config;
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::state::{AppState, TabSession};
use crate::theme::{Palette, Theme};
use crate::ui::file_tree::FileTreeState;
use crate::ui::search_bar::{SearchMode, SearchResult, SearchState};
use crate::ui::tabs::{OpenOutcome, Tabs};
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
    /// All open document tabs (replaces the old single `viewer` field).
    pub tabs: Tabs,
    /// Search overlay state.
    pub search: SearchState,
    /// Go-to-line prompt state (ephemeral — global, not per-tab).
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
    /// Pending first character of a two-key chord (`[` or `]`).
    pub pending_chord: Option<char>,
    /// Per-tab rects in the tab bar, populated during each draw for mouse hit-testing.
    pub tab_bar_rects: Vec<(crate::ui::tabs::TabId, ratatui::layout::Rect)>,
    /// Cached area of the file-tree panel for mouse hit-testing.
    pub tree_area_rect: Option<ratatui::layout::Rect>,
    /// Cached area of the viewer panel for mouse hit-testing.
    pub viewer_area_rect: Option<ratatui::layout::Rect>,
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
            tabs: Tabs::new(),
            search: SearchState::default(),
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
            pending_chord: None,
            tab_bar_rects: Vec::new(),
            tree_area_rect: None,
            viewer_area_rect: None,
        };

        app.restore_session();
        app
    }

    // ── Accessor helpers ─────────────────────────────────────────────────────

    /// Return a shared reference to the active tab's doc-search state, if any tab is open.
    pub fn doc_search(&self) -> Option<&crate::app::DocSearchState> {
        self.tabs.active_tab().map(|t| &t.doc_search)
    }

    /// Return a mutable reference to the active tab's doc-search state, if any tab is open.
    pub fn doc_search_mut(&mut self) -> Option<&mut crate::app::DocSearchState> {
        self.tabs.active_tab_mut().map(|t| &mut t.doc_search)
    }

    // ── Session ──────────────────────────────────────────────────────────────

    /// Restore all tabs from the saved session for the current root directory.
    ///
    /// Each persisted `TabSession` is loaded in order; entries whose files no
    /// longer exist on disk are silently skipped. The saved active index is
    /// clamped to the number of surviving tabs.
    fn restore_session(&mut self) {
        let session = match self.app_state.sessions.get(&self.root).cloned() {
            Some(s) => s,
            None => return,
        };

        let mut last_loaded_path: Option<PathBuf> = None;

        for tab_session in &session.tabs {
            let path = &tab_session.file;
            if path.as_os_str().is_empty() || !path.exists() || !path.starts_with(&self.root) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let scroll = tab_session.scroll;

            let (_, outcome) = self.tabs.open_or_focus(path, true);
            if matches!(outcome, OpenOutcome::Opened | OpenOutcome::Replaced) {
                let tab = self.tabs.active_tab_mut().unwrap();
                tab.view.load(path.clone(), name, content, &self.palette);
                let max_scroll = tab.view.total_lines.saturating_sub(1);
                tab.view.scroll_offset = scroll.min(max_scroll);
            }

            last_loaded_path = Some(path.clone());
        }

        // Activate the saved active index, clamped to surviving tabs.
        let target_active = session.active.min(self.tabs.len().saturating_sub(1));
        self.tabs.activate_by_index(target_active + 1);

        if self.tabs.is_empty() {
            return;
        }

        // Select the active tab's file in the tree.
        let active_path = self.tabs.active_tab().and_then(|t| t.view.current_path.clone());
        let tree_path = active_path.or(last_loaded_path);
        if let Some(path) = tree_path {
            self.expand_and_select(&path);
        }
        self.focus = Focus::Viewer;
    }

    /// Expand every ancestor directory of `file` in the tree and select the file.
    fn expand_and_select(&mut self, file: &PathBuf) {
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

        for (i, item) in self.tree.flat_items.iter().enumerate() {
            if item.path == *file {
                self.tree.list_state.select(Some(i));
                break;
            }
        }
    }

    /// Save all open tabs and the active index to disk.
    fn save_session(&mut self) {
        let tab_sessions: Vec<TabSession> = self
            .tabs
            .tabs
            .iter()
            .filter_map(|t| {
                t.view.current_path.as_ref().map(|p| TabSession {
                    file: p.clone(),
                    scroll: t.view.scroll_offset,
                })
            })
            .collect();

        if tab_sessions.is_empty() {
            return;
        }

        let active_idx = self.tabs.active_index().unwrap_or(0);
        let root = self.root.clone();
        self.app_state.update_session(&root, tab_sessions, active_idx);
    }

    /// Persist the current config settings.
    fn persist_config(&self) {
        Config {
            theme: self.theme,
            show_line_numbers: self.show_line_numbers,
        }
        .save();
    }

    // ── Event loop ───────────────────────────────────────────────────────────

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
            Action::TreeSelect => self.open_in_active_tab(),
            Action::ScrollUp(n) => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_up(n, vh);
                }
            }
            Action::ScrollDown(n) => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_down(n, vh);
                }
            }
            Action::ScrollHalfPageUp => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_half_page_up(vh);
                }
            }
            Action::ScrollHalfPageDown => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_half_page_down(vh);
                }
            }
            Action::ScrollToTop => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_top();
                }
            }
            Action::ScrollToBottom => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_bottom(vh);
                }
            }
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
                if self.tabs.active_tab().and_then(|t| t.view.current_path.as_ref()).is_some() {
                    self.reload_current_tab();
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
            let theme = Theme::ALL[cursor];
            self.theme = theme;
            self.palette = Palette::from_theme(theme);
            self.rerender_all_tabs();
            self.persist_config();
        } else {
            self.show_line_numbers = !self.show_line_numbers;
            self.persist_config();
        }
    }

    /// Re-render every open tab with the active palette, preserving scroll offsets.
    fn rerender_all_tabs(&mut self) {
        let palette = self.palette;
        self.tabs.rerender_all(&palette);
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
                        self.open_in_active_tab();
                    }
                }
            }
            // `t` in the tree opens the selected file in a new tab.
            KeyCode::Char('t') => self.open_selected_file(true),
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
        // Consume and resolve any pending chord first.
        let chord = self.pending_chord.take();

        if let Some(leader) = chord {
            // We have a pending `[` or `]`; see if `t` completes it.
            if code == KeyCode::Char('t') {
                match leader {
                    ']' => self.tabs.next(),
                    '[' => self.tabs.prev(),
                    _ => {}
                }
                return;
            }
            // Chord not completed — fall through and handle the current key
            // normally (the leader is dropped).
        }

        match code {
            KeyCode::Esc => {
                if let Some(ds) = self.doc_search_mut() {
                    ds.active = false;
                    ds.query.clear();
                    ds.match_lines.clear();
                }
            }
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_down(1, vh);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_up(1, vh);
                }
            }
            KeyCode::Char('d') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_half_page_down(vh);
                }
            }
            KeyCode::Char('u') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_half_page_up(vh);
                }
            }
            KeyCode::PageDown => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_page_down(vh);
                }
            }
            KeyCode::PageUp => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_page_up(vh);
                }
            }
            KeyCode::Char('g') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_top();
                }
            }
            KeyCode::Char('G') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_bottom(vh);
                }
            }
            KeyCode::Tab => self.focus = Focus::Tree,
            // Latch the chord leader; `]t` / `[t` will be resolved next keystroke.
            KeyCode::Char('[') => self.pending_chord = Some('['),
            KeyCode::Char(']') => self.pending_chord = Some(']'),
            // `x` closes the active tab.
            KeyCode::Char('x') => {
                if let Some(id) = self.tabs.active {
                    self.tabs.close(id);
                    if self.tabs.is_empty() {
                        self.focus = Focus::Tree;
                    }
                }
            }
            // Backtick jumps to the previously active tab.
            KeyCode::Char('`') => self.tabs.activate_previous(),
            // `1`–`9` jump to that tab by 1-based index; `0` jumps to the last.
            KeyCode::Char('0') => self.tabs.activate_last(),
            KeyCode::Char(c @ '1'..='9') => {
                self.tabs.activate_by_index((c as u8 - b'0') as usize);
            }
            KeyCode::Char('/') => {
                self.search.activate();
                self.focus = Focus::Search;
            }
            KeyCode::Char('f') if modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(ds) = self.doc_search_mut() {
                    ds.active = true;
                    ds.query.clear();
                    ds.match_lines.clear();
                    ds.current_match = 0;
                }
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
                {
                    let tab = self.tabs.active_tab_mut();
                    if let Some(tab) = tab
                        && tab.view.total_lines > 0
                    {
                        let max_line = tab.view.total_lines;
                        tab.view.scroll_offset = n.min(max_line) - 1;
                    }
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
                if let Some(ds) = self.doc_search_mut() {
                    ds.active = false;
                    ds.query.clear();
                    ds.match_lines.clear();
                }
                self.focus = Focus::Viewer;
            }
            KeyCode::Enter => {
                self.focus = Focus::Viewer;
            }
            KeyCode::Backspace => {
                if let Some(ds) = self.doc_search_mut() {
                    ds.query.pop();
                }
                self.perform_doc_search();
            }
            KeyCode::Char(c) => {
                if let Some(ds) = self.doc_search_mut() {
                    ds.query.push(c);
                }
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

    // ── File opening ─────────────────────────────────────────────────────────

    /// Open the selected tree item in the active tab (replacing its content).
    fn open_in_active_tab(&mut self) {
        self.open_selected_file(false);
    }

    /// Open `path` in a tab.
    ///
    /// `new_tab == true` pushes a new tab (or focuses an existing one with the
    /// same path). `new_tab == false` replaces the active tab's content.
    pub fn open_or_focus(&mut self, path: PathBuf, new_tab: bool) {
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

        let (_, outcome) = self.tabs.open_or_focus(&path, new_tab);
        if matches!(outcome, OpenOutcome::Opened | OpenOutcome::Replaced) {
            let palette = self.palette;
            let tab = self.tabs.active_tab_mut().unwrap();
            tab.view.load(path.clone(), name, content, &palette);
        }

        self.focus = Focus::Viewer;
        // Session is persisted on quit via save_session(); no need to write here.
    }

    fn open_selected_file(&mut self, new_tab: bool) {
        let Some(path) = self.tree.selected_path().map(|p| p.to_path_buf()) else {
            return;
        };
        self.open_or_focus(path, new_tab);
    }

    fn reload_current_tab(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let Some(current_path) = tab.view.current_path.clone() else {
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
        let scroll = tab.view.scroll_offset;
        let palette = self.palette;
        let tab = self.tabs.active_tab_mut().unwrap();
        tab.view.load(current_path, name, content, &palette);
        tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));
    }

    // ── Search ───────────────────────────────────────────────────────────────

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

            let (_, outcome) = self.tabs.open_or_focus(&path, true);
            if matches!(outcome, OpenOutcome::Opened | OpenOutcome::Replaced) {
                let palette = self.palette;
                let tab = self.tabs.active_tab_mut().unwrap();
                tab.view.load(path.clone(), name, content, &palette);
            }

            self.search.active = false;
            self.focus = Focus::Viewer;

            for (i, item) in self.tree.flat_items.iter().enumerate() {
                if item.path == result_path {
                    self.tree.list_state.select(Some(i));
                    break;
                }
            }

        }
    }

    fn shrink_tree(&mut self) {
        self.tree_width_pct = self.tree_width_pct.saturating_sub(5).max(10);
    }

    fn grow_tree(&mut self) {
        self.tree_width_pct = (self.tree_width_pct + 5).min(80);
    }

    fn perform_doc_search(&mut self) {
        let query = match self.doc_search() {
            Some(ds) => ds.query.clone(),
            None => return,
        };

        if let Some(ds) = self.doc_search_mut() {
            ds.match_lines.clear();
            ds.current_match = 0;
        }

        if query.is_empty() {
            return;
        }

        let query_lower = query.to_lowercase();

        let tab = match self.tabs.active_tab_mut() {
            Some(t) => t,
            None => return,
        };

        for (i, line) in tab.view.rendered.lines.iter().enumerate() {
            let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if line_text.to_lowercase().contains(&query_lower) {
                tab.doc_search.match_lines.push(i as u32);
            }
        }

        if let Some(&line) = tab.doc_search.match_lines.first() {
            tab.view.scroll_offset = line;
        }
    }

    fn doc_search_next(&mut self) {
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let ds = &mut tab.doc_search;
        if ds.match_lines.is_empty() {
            return;
        }
        ds.current_match = (ds.current_match + 1) % ds.match_lines.len();
        let line = ds.match_lines[ds.current_match];
        tab.view.scroll_offset = line;
    }

    fn doc_search_prev(&mut self) {
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let ds = &mut tab.doc_search;
        if ds.match_lines.is_empty() {
            return;
        }
        ds.current_match = if ds.current_match == 0 {
            ds.match_lines.len() - 1
        } else {
            ds.current_match - 1
        };
        let line = ds.match_lines[ds.current_match];
        tab.view.scroll_offset = line;
    }
}
