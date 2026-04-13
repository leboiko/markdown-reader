use crate::action::Action;
use crate::config::{Config, TreePosition};
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::fs::git_status;
use crate::markdown::DocBlock;
use crate::mermaid::{MermaidCache, MermaidEntry};
use crate::state::{AppState, TabSession};
use crate::theme::{Palette, Theme};
use crate::ui::file_tree::FileTreeState;
use crate::ui::link_picker::{LinkPickerItem, LinkPickerState};
use crate::ui::markdown_view::{TableLayout, visual_row_to_logical_line};
use crate::ui::search_bar::{SearchMode, SearchResult, SearchState};
use crate::ui::tab_picker::TabPickerState;
use crate::ui::tabs::{OpenOutcome, Tabs};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Write `text` to the system clipboard via the OSC 52 terminal escape sequence.
///
/// The sequence is intercepted by the terminal emulator and does not require
/// any external clipboard daemon. It uses the BEL terminator for maximum
/// compatibility across terminals.
fn copy_to_clipboard(text: &str) {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let osc52 = format!("\x1b]52;c;{encoded}\x07");
    let _ = std::io::Write::write_all(&mut std::io::stdout(), osc52.as_bytes());
}

/// Returns `true` when terminal position `(col, row)` falls inside `rect`.
fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Collect absolute display-line numbers whose text matches `query_lower` across
/// all block types in `blocks`.
///
/// Tables: match against the cached fair-share rendered lines so highlights align
/// with what is on screen; fall back to joining raw cell text before the first draw.
///
/// Mermaid: only match when the entry is showing as source (Failed / SourceOnly /
/// absent). Rendered images have no searchable text content. Only the first
/// `MERMAID_BLOCK_HEIGHT - 1` source lines are considered — lines beyond that
/// overflow the fixed block height and are not visible.
pub fn collect_match_lines(
    blocks: &[DocBlock],
    table_layouts: &HashMap<crate::markdown::TableBlockId, TableLayout>,
    mermaid_cache: &MermaidCache,
    query_lower: &str,
) -> Vec<u32> {
    let mut matches = Vec::new();
    let mut offset = 0u32;

    for block in blocks {
        match block {
            DocBlock::Text { text, .. } => {
                for (i, line) in text.lines.iter().enumerate() {
                    let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    if line_text.to_lowercase().contains(query_lower) {
                        matches.push(offset + i as u32);
                    }
                }
                offset += text.lines.len() as u32;
            }
            DocBlock::Table(table) => {
                if let Some(layout) = table_layouts.get(&table.id) {
                    for (i, line) in layout.text.lines.iter().enumerate() {
                        let line_text: String =
                            line.spans.iter().map(|s| s.content.as_ref()).collect();
                        if line_text.to_lowercase().contains(query_lower) {
                            matches.push(offset + i as u32);
                        }
                    }
                } else {
                    // No cached layout yet — fall back to raw cell text so search
                    // is functional before the first draw populates the cache.
                    let mut row_offset = 1u32; // skip top border line
                    let all_rows = std::iter::once(&table.headers).chain(table.rows.iter());
                    for row in all_rows {
                        let row_text: String = row
                            .iter()
                            .map(|cell| crate::markdown::cell_to_string(cell))
                            .collect::<Vec<_>>()
                            .join(" ");
                        if row_text.to_lowercase().contains(query_lower) {
                            matches.push(offset + row_offset);
                        }
                        row_offset += 1;
                    }
                }
                offset += table.rendered_height;
            }
            DocBlock::Mermaid { id, source, .. } => {
                let block_height = block.height();
                let show_as_source = match mermaid_cache.get(id) {
                    None | Some(MermaidEntry::Failed(_)) | Some(MermaidEntry::SourceOnly(_)) => {
                        true
                    }
                    Some(MermaidEntry::Pending) | Some(MermaidEntry::Ready { .. }) => false,
                };
                if show_as_source {
                    let limit = block_height.saturating_sub(1) as usize;
                    for (i, line) in source.lines().take(limit).enumerate() {
                        if line.to_lowercase().contains(query_lower) {
                            matches.push(offset + i as u32);
                        }
                    }
                }
                offset += block_height;
            }
        }
    }

    matches
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
    /// Full-screen table modal (opened with Enter on a table block).
    TableModal,
    /// Copy path/filename menu popup.
    CopyMenu,
    /// Internal-link anchor picker (opened with `f`).
    LinkPicker,
}

/// State for the copy-path popup opened with `y` in the tree.
#[derive(Debug, Clone)]
pub struct CopyMenuState {
    /// 0 = full path, 1 = filename only.
    pub cursor: usize,
    pub path: PathBuf,
    pub name: String,
}

/// State for the full-screen table modal opened with Enter on a table block.
#[derive(Debug, Clone)]
pub struct TableModalState {
    pub tab_id: crate::ui::tabs::TabId,
    pub h_scroll: u16,
    pub v_scroll: u16,
    pub headers: Vec<crate::markdown::CellSpans>,
    pub rows: Vec<Vec<crate::markdown::CellSpans>>,
    pub alignments: Vec<pulldown_cmark::Alignment>,
    pub natural_widths: Vec<usize>,
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
    pub const SECTIONS: &'static [(&'static str, usize)] =
        &[("Theme", Theme::ALL.len()), ("Markdown", 1), ("Panels", 2)];

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
    /// Which side of the screen the file-tree panel appears on.
    pub tree_position: TreePosition,
    /// Copy-path popup state; `None` when the popup is closed.
    pub copy_menu: Option<CopyMenuState>,
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
    /// Link picker overlay state; `None` when the picker is closed.
    pub link_picker: Option<LinkPickerState>,
    /// Cached area of the file-tree panel for mouse hit-testing.
    pub tree_area_rect: Option<ratatui::layout::Rect>,
    /// Cached area of the viewer panel for mouse hit-testing.
    pub viewer_area_rect: Option<ratatui::layout::Rect>,
    /// Cache of mermaid diagram render state, keyed by diagram hash.
    pub mermaid_cache: MermaidCache,
    /// Terminal graphics protocol picker; `None` when graphics are disabled.
    pub picker: Option<Picker>,
    /// State for the full-screen table modal; `None` when the modal is closed.
    pub table_modal: Option<TableModalState>,
    /// Monotonically increasing counter incremented on every new content search.
    ///
    /// Background tasks capture the counter at spawn time and discard their
    /// results silently when it has advanced, preventing stale results from an
    /// older query from overwriting results from a newer one.
    search_generation: Arc<AtomicU64>,
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
        tree.git_status = git_status::collect(&root);

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
            tree_position: config.tree_position,
            copy_menu: None,
            app_state,
            action_tx: None,
            pending_chord: None,
            tab_bar_rects: Vec::new(),
            tab_picker_rects: Vec::new(),
            tab_picker: None,
            link_picker: None,
            tree_area_rect: None,
            viewer_area_rect: None,
            mermaid_cache: MermaidCache::new(),
            picker,
            table_modal: None,
            search_generation: Arc::new(AtomicU64::new(0)),
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

    /// Blocking session save used on quit to ensure data reaches disk before
    /// the process exits.
    fn save_session(&mut self) {
        let Some((mut state, root, tab_sessions, active_idx)) = self.session_snapshot() else {
            return;
        };
        state.update_session(&root, tab_sessions, active_idx);
    }

    /// Build the data needed for a session write without mutating `self`.
    fn session_snapshot(&self) -> Option<(AppState, PathBuf, Vec<TabSession>, usize)> {
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
            return None;
        }

        let active_idx = self.tabs.active_index().unwrap_or(0);
        Some((
            self.app_state.clone(),
            self.root.clone(),
            tab_sessions,
            active_idx,
        ))
    }

    /// Persist the current config settings on a background thread (fire-and-forget).
    fn persist_config(&self) {
        let config = Config {
            theme: self.theme,
            show_line_numbers: self.show_line_numbers,
            tree_position: self.tree_position,
        };
        tokio::task::spawn_blocking(move || config.save());
    }

    /// Re-run `git status` on a background thread and send the result back as
    /// [`Action::GitStatusReady`]. No-ops when `action_tx` is not yet set.
    fn refresh_git_status(&self) {
        let Some(tx) = self.action_tx.clone() else {
            return;
        };
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let map = git_status::collect(&root);
            let _ = tx.send(Action::GitStatusReady(map));
        });
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
            Action::FilesChanged(changed) => {
                let entries = FileEntry::discover(&self.root);
                self.tree.rebuild(entries);
                // Each changed file is read on a background thread; the result
                // arrives as Action::FileReloaded which also handles modal cleanup.
                self.reload_changed_tabs(&changed);
                self.refresh_git_status();
            }
            Action::Resize(_, _) => {}
            Action::Mouse(m) => self.handle_mouse(m),
            Action::MermaidReady(id, entry) => {
                self.mermaid_cache.insert(id, *entry);
            }
            Action::SearchResults {
                generation,
                results,
            } => {
                // Discard if a newer search has already been started.
                if self.search_generation.load(Ordering::Relaxed) == generation {
                    self.search.results = results;
                    self.search.selected_index = 0;
                }
            }
            Action::FileLoaded {
                path,
                content,
                new_tab,
            } => {
                self.apply_file_loaded(path, content, new_tab);
            }
            Action::FileReloaded { path, content } => {
                self.apply_file_reloaded(path, content);
            }
            Action::GitStatusReady(map) => {
                self.tree.git_status = map;
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
                    self.close_table_modal();
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
                    self.close_table_modal();
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
                    self.try_follow_link_click(viewer_rect, col, row);
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

    /// If the click coordinates land on an internal `#anchor` link, scroll to
    /// the matching heading. External links are ignored silently.
    ///
    /// `viewer_rect` is the outer border rect of the viewer panel; the inner
    /// content area starts one cell inside on each side.
    fn try_follow_link_click(&mut self, viewer_rect: ratatui::layout::Rect, col: u16, row: u16) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        // The content inner rect (inside the 1-cell border).
        let inner_x = viewer_rect.x + 1;
        let inner_y = viewer_rect.y + 1;

        if row < inner_y || col < inner_x {
            return;
        }

        let scroll_offset = tab.view.scroll_offset;
        let visual_row = (row - inner_y) as u32;

        // Subtract the gutter width when line numbers are shown. The formula
        // matches render_text_with_gutter so click positions align with text.
        let content_col = if self.show_line_numbers {
            let total_lines = tab.view.total_lines.max(10);
            let num_digits = (total_lines.ilog10() + 1).max(4) as u16;
            let gutter_width = num_digits + 3;
            (col - inner_x).saturating_sub(gutter_width)
        } else {
            col - inner_x
        };

        // `layout_width` is the text content width (excluding the gutter).
        // `Paragraph::wrap` wraps at this width, so logical lines that are
        // wider than `layout_width` occupy multiple visual rows. We must
        // account for this wrapping to convert the clicked visual row back to
        // the correct logical document line.
        let content_width = tab.view.layout_width;
        let clicked_line = visual_row_to_logical_line(
            &tab.view.rendered,
            scroll_offset,
            visual_row,
            content_width,
        );

        let anchor = tab
            .view
            .links
            .iter()
            .find(|l| {
                l.line == clicked_line
                    && content_col >= l.col_start
                    && content_col < l.col_end
                    && l.url.starts_with('#')
            })
            .map(|l| l.url[1..].to_string());

        if let Some(anchor) = anchor {
            let target_line = tab
                .view
                .heading_anchors
                .iter()
                .find(|a| a.anchor == anchor)
                .map(|a| a.line);
            if let Some(line) = target_line {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    let max = tab.view.total_lines.saturating_sub(vh / 2);
                    // Show 2 lines of context above the heading so it doesn't
                    // land flush at the viewport edge.
                    tab.view.scroll_offset = line.saturating_sub(2).min(max);
                }
            }
        }
    }

    /// Build the link picker from the active tab's internal `#anchor` links,
    /// deduplicated by anchor, and open it.
    fn open_link_picker(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        // Collect unique anchors preserving first-occurrence order.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut items: Vec<LinkPickerItem> = Vec::new();

        for link in &tab.view.links {
            if !link.url.starts_with('#') {
                continue;
            }
            let anchor = &link.url[1..];
            if !seen.insert(anchor.to_string()) {
                continue;
            }
            // Only include links that resolve to a known heading anchor.
            let has_target = tab.view.heading_anchors.iter().any(|a| a.anchor == anchor);
            if has_target {
                items.push(LinkPickerItem {
                    text: link.text.clone(),
                    anchor: anchor.to_string(),
                });
            }
        }

        if items.is_empty() {
            return;
        }

        self.link_picker = Some(LinkPickerState { cursor: 0, items });
        self.focus = Focus::LinkPicker;
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

        if code == KeyCode::Char('H')
            && self.focus != Focus::Search
            && self.focus != Focus::TableModal
        {
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
                if self.tab_picker.is_none() {
                    self.focus = Focus::Viewer;
                }
            }
            Focus::LinkPicker => {
                crate::ui::link_picker::handle_key(self, code);
                if self.link_picker.is_none() {
                    self.focus = Focus::Viewer;
                }
            }
            Focus::TableModal => {
                self.handle_table_modal_key(code);
            }
            Focus::CopyMenu => self.handle_copy_menu_key(code),
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
        } else if cursor == theme_count {
            self.show_line_numbers = !self.show_line_numbers;
            self.persist_config();
        } else {
            // Panels section: index 0 = Tree left, index 1 = Tree right.
            let panels_cursor = cursor - theme_count - 1;
            self.tree_position = if panels_cursor == 0 {
                TreePosition::Left
            } else {
                TreePosition::Right
            };
            self.persist_config();
        }
    }

    /// Re-render every open tab with the active palette, preserving scroll offsets.
    fn rerender_all_tabs(&mut self) {
        let palette = self.palette;
        self.tabs.rerender_all(&palette);
        // Mermaid images have the theme background baked into their pixels,
        // so they must re-render when the theme changes.
        self.mermaid_cache.clear();
    }

    /// Commit any in-progress doc-search and switch focus back to Viewer before
    /// performing a tab switch.
    fn commit_doc_search_if_active(&mut self) {
        if self.focus == Focus::DocSearch {
            self.focus = Focus::Viewer;
        }
    }

    /// Close the table modal if open, restoring focus to Viewer.
    pub fn close_table_modal(&mut self) {
        if self.table_modal.is_some() {
            self.table_modal = None;
            self.focus = Focus::Viewer;
        }
    }

    fn switch_to_next_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
        self.tabs.next();
    }

    fn switch_to_prev_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
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
            KeyCode::Char('y') => {
                if let Some(item) = self.tree.selected_item() {
                    self.copy_menu = Some(CopyMenuState {
                        cursor: 0,
                        path: item.path.clone(),
                        name: item.name.clone(),
                    });
                    self.focus = Focus::CopyMenu;
                }
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
            KeyCode::Enter => {
                self.try_open_table_modal();
            }
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
                self.close_table_modal();
                self.tabs.activate_previous();
            }
            // `1`–`9` jump to that tab by 1-based index; `0` jumps to the last.
            KeyCode::Char('0') => {
                self.commit_doc_search_if_active();
                self.close_table_modal();
                self.tabs.activate_last();
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.commit_doc_search_if_active();
                self.close_table_modal();
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
            KeyCode::Char('f') => {
                self.open_link_picker();
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

    fn handle_copy_menu_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(m) = self.copy_menu.as_mut() {
                    m.cursor = m.cursor.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = self.copy_menu.as_mut() {
                    m.cursor = (m.cursor + 1).min(1);
                }
            }
            KeyCode::Enter => {
                if let Some(m) = &self.copy_menu {
                    let text = if m.cursor == 0 {
                        m.path.to_string_lossy().to_string()
                    } else {
                        m.name.clone()
                    };
                    copy_to_clipboard(&text);
                }
                self.copy_menu = None;
                self.focus = Focus::Tree;
            }
            KeyCode::Esc | KeyCode::Char('y') => {
                self.copy_menu = None;
                self.focus = Focus::Tree;
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
    ///
    /// If the file is already open the switch is instantaneous (no I/O).
    /// Otherwise the read is dispatched to a background thread and the result
    /// arrives as [`Action::FileLoaded`].
    pub fn open_or_focus(&mut self, path: PathBuf, new_tab: bool) {
        if path.is_dir() {
            return;
        }

        // If the tab already exists, activating it requires no disk I/O.
        let (_, outcome) = self.tabs.open_or_focus(&path, new_tab);
        if matches!(outcome, OpenOutcome::Focused) {
            self.focus = Focus::Viewer;
            return;
        }

        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        tokio::task::spawn_blocking(move || {
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };
            let _ = tx.send(Action::FileLoaded {
                path,
                content,
                new_tab,
            });
        });

        self.focus = Focus::Viewer;
    }

    /// Apply a completed async file load: populate the tab that was reserved
    /// by [`open_or_focus`].
    fn apply_file_loaded(&mut self, path: PathBuf, content: String, _new_tab: bool) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Find the placeholder tab that open_or_focus reserved (it has
        // current_path set but no content yet) and load the file into it.
        let palette = self.palette;
        let loaded = self
            .tabs
            .find_tab_by_path_mut(&path)
            .filter(|t| t.view.content.is_empty())
            .is_some();

        if loaded {
            let tab = self.tabs.find_tab_by_path_mut(&path).unwrap();
            tab.view.load(path.clone(), name, content, &palette);
        }

        self.focus = Focus::Viewer;
        self.expand_and_select(&path);
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
    /// Spawn a background read for each tab whose path is in `changed`.
    ///
    /// Each read completes asynchronously and arrives as [`Action::FileReloaded`].
    fn reload_changed_tabs(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() || self.tabs.is_empty() {
            return;
        }

        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        for tab in &self.tabs.tabs {
            let Some(path) = tab.view.current_path.clone() else {
                continue;
            };
            if !changed.contains(&path) {
                continue;
            }
            let tx = tx.clone();
            tokio::task::spawn_blocking(move || {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    return;
                };
                let _ = tx.send(Action::FileReloaded { path, content });
            });
        }
    }

    /// Apply a completed async file reload to every tab open on `path`.
    ///
    /// Preserves each tab's scroll offset (clamped to the new line count).
    fn apply_file_reloaded(&mut self, path: PathBuf, content: String) {
        let palette = self.palette;

        for tab in &mut self.tabs.tabs {
            if tab.view.current_path.as_deref() != Some(&*path) {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let scroll = tab.view.scroll_offset;
            tab.view.load(path.clone(), name, content.clone(), &palette);
            tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));
        }

        // Drop cache entries for mermaid blocks that no longer exist after the
        // reload. Fresh DocBlock::Mermaid values get new ids from their content
        // hash, making old cache entries permanently stale.
        let alive: std::collections::HashSet<crate::markdown::MermaidBlockId> = self
            .tabs
            .tabs
            .iter()
            .flat_map(|t| t.view.rendered.iter())
            .filter_map(|b| match b {
                crate::markdown::DocBlock::Mermaid { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        self.mermaid_cache.retain(&alive);

        // Close the table modal if it was open on the reloaded tab.
        if let Some(modal) = &self.table_modal {
            let tab_id = modal.tab_id;
            let is_reloaded = self
                .tabs
                .tabs
                .iter()
                .any(|t| t.id == tab_id && t.view.current_path.as_deref() == Some(&*path));
            if is_reloaded {
                self.close_table_modal();
            }
        }
    }

    // ── Search ───────────────────────────────────────────────────────────────

    fn perform_search(&mut self) {
        // Clear stale results immediately so the UI never shows results from a
        // superseded query while the new background task is running.
        self.search.results.clear();
        self.search.selected_index = 0;

        if self.search.query.is_empty() {
            return;
        }

        let query_lower = self.search.query.to_lowercase();

        match self.search.mode {
            SearchMode::FileName => {
                // Filename search is O(n) over in-memory data — fast enough to
                // run synchronously on the main thread with no perceptible delay.
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
                // Content search reads every file on disk — offload to a blocking
                // thread so the event loop remains responsive during the scan.
                let Some(tx) = self.action_tx.clone() else {
                    return;
                };

                // Advance the generation counter. The spawned task captures this
                // generation; if it has been superseded by the time it finishes
                // it will discard its results without sending to the channel.
                let generation = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
                let gen_arc = Arc::clone(&self.search_generation);

                let paths = FileEntry::flat_paths(&self.tree.entries);

                tokio::task::spawn_blocking(move || {
                    let mut results = Vec::new();
                    for path in paths {
                        // Bail early if a newer search has already started.
                        if gen_arc.load(Ordering::Relaxed) != generation {
                            return;
                        }
                        let Ok(content) = std::fs::read_to_string(&path) else {
                            continue;
                        };
                        for (i, line) in content.lines().enumerate() {
                            if line.to_lowercase().contains(&query_lower) {
                                let name = path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string();
                                results.push(SearchResult {
                                    path: path.clone(),
                                    name,
                                    line_number: Some(i + 1),
                                    snippet: Some(line.trim().to_string()),
                                });
                                break;
                            }
                        }
                    }
                    // Final check before sending — another keystroke may have
                    // arrived while we were iterating the last file.
                    if gen_arc.load(Ordering::Relaxed) == generation {
                        let _ = tx.send(Action::SearchResults {
                            generation,
                            results,
                        });
                    }
                });
            }
        }
    }

    fn confirm_search(&mut self) {
        let Some(result) = self.search.results.get(self.search.selected_index).cloned() else {
            return;
        };

        // Close the search overlay immediately so the UI is not frozen waiting
        // for the read. The file content arrives via Action::FileLoaded.
        self.search.active = false;
        self.focus = Focus::Viewer;

        for (i, item) in self.tree.flat_items.iter().enumerate() {
            if item.path == result.path {
                self.tree.list_state.select(Some(i));
                break;
            }
        }

        self.open_or_focus(result.path, true);
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

        let match_lines = collect_match_lines(
            &tab.view.rendered,
            &tab.view.table_layouts,
            &self.mermaid_cache,
            &query_lower,
        );
        tab.doc_search.match_lines = match_lines;

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

    // ── Table modal ──────────────────────────────────────────────────────────

    /// Open the table modal if the block at the viewport center is a table.
    fn try_open_table_modal(&mut self) {
        let view_height = self.tabs.view_height;
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let viewport_start = tab.view.scroll_offset;
        let viewport_end = viewport_start + view_height;

        // Expand the first table whose range intersects the viewport. Viewport
        // center detection would miss the common case of a table sitting at
        // the top or bottom of the visible area with the center on surrounding
        // prose.
        let mut block_start = 0u32;
        for doc_block in &tab.view.rendered {
            let block_end = block_start + doc_block.height();
            let intersects = block_end > viewport_start && block_start < viewport_end;

            if intersects && let crate::markdown::DocBlock::Table(table) = doc_block {
                let modal = TableModalState {
                    tab_id: tab.id,
                    h_scroll: 0,
                    v_scroll: 0,
                    headers: table.headers.clone(),
                    rows: table.rows.clone(),
                    alignments: table.alignments.clone(),
                    natural_widths: table.natural_widths.clone(),
                };
                self.table_modal = Some(modal);
                self.focus = Focus::TableModal;
                return;
            }

            block_start = block_end;
            if block_start >= viewport_end {
                break;
            }
        }
    }

    fn handle_table_modal_key(&mut self, code: KeyCode) {
        use crate::ui::table_modal::max_h_scroll;

        if self.pending_chord.take() == Some('g') && code == KeyCode::Char('g') {
            if let Some(s) = self.table_modal.as_mut() {
                s.v_scroll = 0;
                s.h_scroll = 0;
            }
            return;
        }

        let view_height = self.tabs.view_height as u16;

        match code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => {
                self.close_table_modal();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(s) = self.table_modal.as_mut() {
                    let max = max_h_scroll(s, view_height);
                    s.h_scroll = (s.h_scroll + 1).min(max);
                }
            }
            KeyCode::Char('H') => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(10);
                }
            }
            KeyCode::Char('L') => {
                if let Some(s) = self.table_modal.as_mut() {
                    let max = max_h_scroll(s, view_height);
                    s.h_scroll = (s.h_scroll + 10).min(max);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('d') | KeyCode::PageDown => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll += view_height / 2;
                }
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(view_height / 2);
                }
            }
            KeyCode::Char('G') => {
                if let Some(s) = self.table_modal.as_mut() {
                    // Jump to bottom: rows + 3 border lines - 1.
                    let total = s.rows.len() as u16 + 3;
                    s.v_scroll = total.saturating_sub(view_height);
                }
            }
            KeyCode::Char('0') => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = 0;
                }
            }
            KeyCode::Char('$') => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = max_h_scroll(s, view_height);
                }
            }
            KeyCode::Char('g') => {
                self.pending_chord = Some('g');
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{CellSpans, MermaidBlockId, TableBlock, TableBlockId};
    use crate::mermaid::{DEFAULT_MERMAID_HEIGHT, MermaidEntry};
    use crate::ui::markdown_view::TableLayout;
    use ratatui::text::{Line, Span, Text};
    use std::cell::Cell;

    fn make_text_block(lines: &[&str]) -> DocBlock {
        let text_lines: Vec<Line<'static>> = lines
            .iter()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        DocBlock::Text {
            text: Text::from(text_lines),
            links: Vec::new(),
            heading_anchors: Vec::new(),
        }
    }

    fn str_cell(s: &str) -> CellSpans {
        vec![Span::raw(s.to_string())]
    }

    fn make_table_block(id: u64, headers: &[&str], rows: &[&[&str]]) -> DocBlock {
        let h: Vec<CellSpans> = headers.iter().map(|s| str_cell(s)).collect();
        let r: Vec<Vec<CellSpans>> = rows
            .iter()
            .map(|row| row.iter().map(|s| str_cell(s)).collect())
            .collect();
        let num_cols = h.len();
        let natural_widths = vec![10usize; num_cols];
        DocBlock::Table(TableBlock {
            id: TableBlockId(id),
            headers: h,
            rows: r,
            alignments: vec![pulldown_cmark::Alignment::None; num_cols],
            natural_widths,
            rendered_height: 4,
        })
    }

    fn make_cached_layout(lines: &[&str]) -> TableLayout {
        let text_lines: Vec<Line<'static>> = lines
            .iter()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        TableLayout {
            text: Text::from(text_lines),
        }
    }

    fn empty_mermaid_cache() -> MermaidCache {
        MermaidCache::new()
    }

    fn source_only_cache(id: u64) -> MermaidCache {
        let mut cache = MermaidCache::new();
        cache.insert(
            MermaidBlockId(id),
            MermaidEntry::SourceOnly("test".to_string()),
        );
        cache
    }

    fn ready_cache(id: u64) -> MermaidCache {
        // We can't build a StatefulProtocol in tests, so we use Failed as a
        // stand-in for "showing as image" — which would normally suppress search.
        // For the Ready variant specifically we use Failed to confirm the negative
        // (Failed does show source). Use a separate test for the suppression path.
        let mut cache = MermaidCache::new();
        cache.insert(
            MermaidBlockId(id),
            MermaidEntry::Failed("irrelevant".to_string()),
        );
        cache
    }

    #[test]
    fn collect_matches_text_block() {
        let blocks = vec![make_text_block(&["hello world", "no match", "world again"])];
        let layouts = HashMap::new();
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "world");
        assert_eq!(result, vec![0, 2]);
    }

    #[test]
    fn collect_matches_table_with_layout_cache() {
        let blocks = vec![
            make_text_block(&["intro"]),
            make_table_block(1, &["Header"], &[&["alpha"], &["beta needle"]]),
        ];
        let mut layouts = HashMap::new();
        layouts.insert(
            TableBlockId(1),
            make_cached_layout(&[
                "┌──────┐",
                "│ Header │",
                "├──────┤",
                "│ alpha  │",
                "│ beta needle │",
                "└──────┘",
            ]),
        );
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "needle");
        // text block has 1 line (offset 0); table starts at offset 1.
        // "beta needle" is at layout index 4, so absolute = 1 + 4 = 5.
        assert_eq!(result, vec![5]);
    }

    #[test]
    fn collect_matches_table_fallback_no_layout() {
        let blocks = vec![make_table_block(2, &["Col"], &[&["findme"], &["nothing"]])];
        let layouts = HashMap::new();
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "findme");
        // Fallback: header row is at row_offset=1, data rows follow.
        // "findme" is the first data row → row_offset = 2 → absolute = 0+2 = 2.
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn collect_matches_mermaid_source_only() {
        let source = "graph LR\n    A --> needle\n    B --> C";
        let mermaid_id = MermaidBlockId(99);
        let blocks = vec![
            make_text_block(&["before"]),
            DocBlock::Mermaid {
                id: mermaid_id,
                source: source.to_string(),
                cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
            },
        ];
        let cache = source_only_cache(99);
        let layouts = HashMap::new();
        let result = collect_match_lines(&blocks, &layouts, &cache, "needle");
        // text block: 1 line (offset 0). mermaid starts at offset 1.
        // "A --> needle" is source line index 1, so absolute = 1 + 1 = 2.
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn collect_matches_mermaid_failed_shows_source() {
        let mermaid_id = MermaidBlockId(42);
        let blocks = vec![DocBlock::Mermaid {
            id: mermaid_id,
            source: "graph LR\n    find_this".to_string(),
            cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
        }];
        let cache = ready_cache(42);
        let layouts = HashMap::new();
        let result = collect_match_lines(&blocks, &layouts, &cache, "find_this");
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn collect_matches_mermaid_absent_shows_source() {
        let mermaid_id = MermaidBlockId(7);
        let blocks = vec![DocBlock::Mermaid {
            id: mermaid_id,
            source: "sequenceDiagram\n    A ->> match_me: call".to_string(),
            cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
        }];
        let layouts = HashMap::new();
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "match_me");
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn collect_matches_absolute_offsets_across_blocks() {
        let blocks = vec![
            make_text_block(&["line0", "line1", "line2"]),
            make_table_block(5, &["H"], &[&["row0"], &["row1 target"]]),
            make_text_block(&["after"]),
        ];
        let mut layouts = HashMap::new();
        layouts.insert(
            TableBlockId(5),
            make_cached_layout(&["┌─┐", "│H│", "├─┤", "│row0│", "│row1 target│", "└─┘"]),
        );
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "target");
        // text block: 3 lines (offsets 0–2). table starts at 3, rendered_height=4.
        // "row1 target" is at layout index 4 → absolute = 3+4 = 7.
        // after block starts at 3+4=7. "after" is at 7+0=7 — no match for "target".
        assert_eq!(result, vec![7]);
    }
}
