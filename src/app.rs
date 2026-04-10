use crate::action::Action;
use crate::config::Config;
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::mermaid::MermaidCache;
use crate::state::{AppState, TabSession};
use crate::theme::{Palette, Theme};
use crate::ui::file_tree::FileTreeState;
use crate::ui::search_bar::{SearchMode, SearchResult, SearchState};
use crate::ui::tab_picker::TabPickerState;
use crate::ui::tabs::{OpenOutcome, Tabs};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use std::path::PathBuf;

/// Returns `true` when terminal position `(col, row)` falls inside `rect`.
fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

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
    /// Tab picker overlay.
    TabPicker,
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
    /// Per-row rects in the tab picker overlay for mouse hit-testing.
    pub tab_picker_rects: Vec<(crate::ui::tabs::TabId, ratatui::layout::Rect)>,
    /// Tab picker overlay state; `None` when the picker is closed.
    pub tab_picker: Option<TabPickerState>,
    /// Cached area of the file-tree panel for mouse hit-testing.
    pub tree_area_rect: Option<ratatui::layout::Rect>,
    /// Cached area of the viewer panel for mouse hit-testing.
    pub viewer_area_rect: Option<ratatui::layout::Rect>,
    /// Cache of mermaid diagram render state, keyed by diagram hash.
    pub mermaid_cache: MermaidCache,
    /// Terminal graphics protocol picker; `None` when graphics are disabled.
    pub picker: Option<Picker>,
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

        let picker = crate::mermaid::create_picker();

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
            tab_picker_rects: Vec::new(),
            tab_picker: None,
            tree_area_rect: None,
            viewer_area_rect: None,
            mermaid_cache: MermaidCache::new(),
            picker,
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

        // Mermaid queuing requires action_tx, which isn't set yet at
        // restore_session time. Diagrams will be queued on first file open.

        // Select the active tab's file in the tree.
        let active_path = self
            .tabs
            .active_tab()
            .and_then(|t| t.view.current_path.clone());
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
        self.app_state
            .update_session(&root, tab_sessions, active_idx);
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

        // Queue renders for any tabs that were restored from session before
        // action_tx was available.
        self.queue_mermaid_for_all_tabs();

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
            Action::FilesChanged(changed) => {
                let entries = FileEntry::discover(&self.root);
                self.tree.rebuild(entries);
                self.reload_changed_tabs(&changed);
            }
            Action::Resize(_, _) => {}
            Action::Mouse(m) => self.handle_mouse(m),
            Action::MermaidReady(id, entry) => {
                self.mermaid_cache.insert(id, *entry);
            }
        }
    }

    fn handle_mouse(&mut self, m: crossterm::event::MouseEvent) {
        let col = m.column;
        let row = m.row;

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Tab picker rows take priority when the picker is open.
                let picker_hit = self
                    .tab_picker_rects
                    .iter()
                    .find(|(_, rect)| contains(*rect, col, row))
                    .map(|(id, _)| *id);

                if let Some(id) = picker_hit {
                    self.tabs.set_active(id);
                    self.tab_picker = None;
                    self.focus = Focus::Viewer;
                    return;
                }

                // Tab bar click.
                let tab_hit = self
                    .tab_bar_rects
                    .iter()
                    .find(|(_, rect)| contains(*rect, col, row))
                    .map(|(id, _)| *id);

                if let Some(id) = tab_hit {
                    self.commit_doc_search_if_active();
                    self.tabs.set_active(id);
                    self.focus = Focus::Viewer;
                    return;
                }

                // Tree click.
                if let Some(tree_rect) = self.tree_area_rect
                    && contains(tree_rect, col, row)
                {
                    self.focus = Focus::Tree;
                    // The List widget renders items inside the block border.
                    // inner.y = tree_rect.y + 1 (top border).
                    let inner_y = tree_rect.y + 1;
                    if row >= inner_y {
                        let viewport_row = (row - inner_y) as usize;
                        let offset = self.tree.list_state.offset();
                        let idx = offset + viewport_row;
                        if idx < self.tree.flat_items.len() {
                            self.tree.list_state.select(Some(idx));
                            let item = self.tree.flat_items[idx].clone();
                            if item.is_dir {
                                self.tree.toggle_expand();
                            } else {
                                self.open_in_active_tab();
                            }
                        }
                    }
                    return;
                }

                // Viewer click.
                if let Some(viewer_rect) = self.viewer_area_rect
                    && contains(viewer_rect, col, row)
                {
                    self.focus = Focus::Viewer;
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(viewer_rect) = self.viewer_area_rect
                    && contains(viewer_rect, col, row)
                {
                    let vh = self.tabs.view_height;
                    if let Some(tab) = self.tabs.active_tab_mut() {
                        tab.view.scroll_down(3, vh);
                    }
                } else if let Some(tree_rect) = self.tree_area_rect
                    && contains(tree_rect, col, row)
                {
                    self.tree.move_down();
                    self.tree.move_down();
                    self.tree.move_down();
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(viewer_rect) = self.viewer_area_rect
                    && contains(viewer_rect, col, row)
                {
                    let vh = self.tabs.view_height;
                    if let Some(tab) = self.tabs.active_tab_mut() {
                        tab.view.scroll_up(3, vh);
                    }
                } else if let Some(tree_rect) = self.tree_area_rect
                    && contains(tree_rect, col, row)
                {
                    self.tree.move_up();
                    self.tree.move_up();
                    self.tree.move_up();
                }
            }
            _ => {}
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
            Focus::TabPicker => {
                crate::ui::tab_picker::handle_key(self, code);
                // Sync focus if picker was closed.
                if self.tab_picker.is_none() {
                    self.focus = Focus::Viewer;
                }
            }
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

    /// Commit any in-progress doc-search and switch focus back to Viewer before
    /// performing a tab switch.
    ///
    /// The plan requires: "if the user is mid-typing in Find (Focus::DocSearch)
    /// and a tab switch happens, commit the current query to the active tab's
    /// doc_search, return focus to Viewer, then perform the switch."
    fn commit_doc_search_if_active(&mut self) {
        if self.focus == Focus::DocSearch {
            self.focus = Focus::Viewer;
        }
    }

    fn switch_to_next_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.tabs.next();
    }

    fn switch_to_prev_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.tabs.prev();
    }

    fn resolve_g_chord_tree(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('g') => {
                self.tree.go_first();
                true
            }
            KeyCode::Char('t') => {
                self.switch_to_next_tab();
                true
            }
            KeyCode::Char('T') => {
                self.switch_to_prev_tab();
                true
            }
            _ => false,
        }
    }

    fn resolve_g_chord_viewer(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('g') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_top();
                }
                true
            }
            KeyCode::Char('t') => {
                self.switch_to_next_tab();
                true
            }
            KeyCode::Char('T') => {
                self.switch_to_prev_tab();
                true
            }
            _ => false,
        }
    }

    fn handle_tree_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        if self.pending_chord.take() == Some('g') && self.resolve_g_chord_tree(code) {
            return;
        }

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
            KeyCode::Char('g') => self.pending_chord = Some('g'),
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
        // Resolve a pending vim `g` chord before normal dispatch.
        if self.pending_chord.take() == Some('g') && self.resolve_g_chord_viewer(code) {
            return;
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
            KeyCode::Char('g') => self.pending_chord = Some('g'),
            KeyCode::Char('G') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.scroll_to_bottom(vh);
                }
            }
            KeyCode::Tab => self.focus = Focus::Tree,
            KeyCode::Char('[') => self.shrink_tree(),
            KeyCode::Char(']') => self.grow_tree(),
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
            KeyCode::Char('`') => {
                self.commit_doc_search_if_active();
                self.tabs.activate_previous();
            }
            // `1`–`9` jump to that tab by 1-based index; `0` jumps to the last.
            KeyCode::Char('0') => {
                self.commit_doc_search_if_active();
                self.tabs.activate_last();
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.commit_doc_search_if_active();
                self.tabs.activate_by_index((c as u8 - b'0') as usize);
            }
            // `T` opens the tab picker overlay.
            KeyCode::Char('T') => {
                if !self.tabs.is_empty() {
                    let cursor = self.tabs.active_index().unwrap_or(0);
                    self.tab_picker = Some(TabPickerState { cursor });
                    self.focus = Focus::TabPicker;
                }
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

        self.queue_mermaid_for_active_tab();
        self.focus = Focus::Viewer;
    }

    fn open_selected_file(&mut self, new_tab: bool) {
        let Some(path) = self.tree.selected_path().map(|p| p.to_path_buf()) else {
            return;
        };
        self.open_or_focus(path, new_tab);
    }

    /// Reload every open tab whose path is in the `changed` set.
    ///
    /// Preserves each tab's scroll offset (clamped to the new line count).
    /// This replaces `reload_current_tab` for the `FilesChanged` handler.
    fn reload_changed_tabs(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() || self.tabs.is_empty() {
            return;
        }

        let palette = self.palette;

        for tab in &mut self.tabs.tabs {
            let Some(path) = tab.view.current_path.clone() else {
                continue;
            };
            if !changed.contains(&path) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let scroll = tab.view.scroll_offset;
            tab.view.load(path, name, content, &palette);
            tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));
        }

        self.queue_mermaid_for_all_tabs();
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

        for (block_start, text) in tab.view.text_blocks() {
            for (i, line) in text.lines.iter().enumerate() {
                let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                if line_text.to_lowercase().contains(&query_lower) {
                    tab.doc_search.match_lines.push(block_start + i as u32);
                }
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

    // ── Mermaid ──────────────────────────────────────────────────────────────

    /// Queue background renders for all mermaid diagrams in the active tab.
    fn queue_mermaid_for_active_tab(&mut self) {
        let Some(tx) = self.action_tx.clone() else {
            return;
        };
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        let diagrams: Vec<(crate::markdown::MermaidBlockId, String)> = tab
            .view
            .rendered
            .iter()
            .filter_map(|b| match b {
                crate::markdown::DocBlock::Mermaid { id, source } => Some((*id, source.clone())),
                crate::markdown::DocBlock::Text(_) | crate::markdown::DocBlock::Table(_) => None,
            })
            .collect();

        let in_tmux = std::env::var("TMUX").is_ok();
        for (id, source) in diagrams {
            self.mermaid_cache
                .ensure_queued(id, &source, self.picker.as_ref(), &tx, in_tmux);
        }
    }

    fn queue_mermaid_for_all_tabs(&mut self) {
        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        let diagrams: Vec<(crate::markdown::MermaidBlockId, String)> = self
            .tabs
            .tabs
            .iter()
            .flat_map(|t| t.view.rendered.iter())
            .filter_map(|b| match b {
                crate::markdown::DocBlock::Mermaid { id, source } => Some((*id, source.clone())),
                crate::markdown::DocBlock::Text(_) | crate::markdown::DocBlock::Table(_) => None,
            })
            .collect();

        let in_tmux = std::env::var("TMUX").is_ok();
        for (id, source) in diagrams {
            self.mermaid_cache
                .ensure_queued(id, &source, self.picker.as_ref(), &tx, in_tmux);
        }
    }
}
