use crate::action::Action;
use crate::config::{Config, SearchPreview, TreePosition};
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::fs::git_status;
use crate::markdown::DocBlock;
use crate::mermaid::{MermaidCache, MermaidEntry};
use crate::state::{AppState, TabSession};
use crate::theme::{Palette, Theme};
use crate::ui::editor::{
    CommandOutcome, TabEditor, dispatch_command, extract_text, forward_key_to_edtui,
};
use crate::ui::file_tree::FileTreeState;
use crate::ui::link_picker::{LinkPickerItem, LinkPickerState};
use crate::ui::markdown_view::{TableLayout, visual_row_to_logical_line};
use crate::ui::search_modal::{
    RESULT_CAP, SearchMode, SearchResult, SearchState, build_preview, smartcase_is_sensitive,
};
use crate::ui::tab_picker::TabPickerState;
use crate::ui::tabs::{OpenOutcome, Tabs};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use edtui::EditorMode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

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

/// Build the text that `yy` or a visual-mode `y` would copy.
///
/// Selects source lines `start_source..=end_source` (inclusive, 0-indexed)
/// from `content` and joins them with newlines.  The range is normalised so
/// callers need not order the endpoints.  If the range extends past EOF, only
/// the available lines are returned; the function never panics.
///
/// This is a pure helper so the yank logic can be unit-tested without a terminal.
///
/// # Arguments
///
/// * `content`      – raw markdown source.
/// * `start_source` – 0-indexed source line at one end of the selection.
/// * `end_source`   – 0-indexed source line at the other end (may be equal to
///   `start_source` for a single-line yank).
pub(crate) fn build_yank_text(content: &str, start_source: u32, end_source: u32) -> String {
    let (lo, hi) = if start_source <= end_source {
        (start_source as usize, end_source as usize)
    } else {
        (end_source as usize, start_source as usize)
    };
    content
        .lines()
        .skip(lo)
        .take(hi - lo + 1)
        .collect::<Vec<_>>()
        .join("\n")
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
    /// Vim-style in-place editor for the active tab's source file.
    Editor,
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
    ///
    /// The order and counts here must stay in sync with `apply_config_selection`
    /// and `config_popup::build_lines`.
    pub const SECTIONS: &'static [(&'static str, usize)] = &[
        ("Theme", Theme::ALL.len()),
        ("Markdown", 1),
        ("Panels", 2),
        ("Search", 2),
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
    /// Which side of the screen the file-tree panel appears on.
    pub tree_position: TreePosition,
    /// How to render the inline preview in content-search results.
    pub search_preview: SearchPreview,
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
    pub tab_close_rects: Vec<(crate::ui::tabs::TabId, ratatui::layout::Rect)>,
    /// Per-row rects in the tab picker overlay for mouse hit-testing.
    pub tab_picker_rects: Vec<(crate::ui::tabs::TabId, ratatui::layout::Rect)>,
    /// Per-row rects in the search modal for mouse hit-testing.
    ///
    /// Each element is `(result_index, rect)`.  Populated during each draw;
    /// cleared at the start of `search_modal::draw`.
    pub search_result_rects: Vec<(usize, ratatui::layout::Rect)>,
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
    /// Cached outer rect of the table modal popup, populated each draw frame.
    ///
    /// Used by the mouse handler to hit-test clicks and scroll events against the
    /// modal boundary. Cleared in `close_table_modal`.
    pub table_modal_rect: Option<ratatui::layout::Rect>,
    /// Monotonically increasing counter incremented on every new content search.
    ///
    /// Background tasks capture the counter at spawn time and discard their
    /// results silently when it has advanced, preventing stale results from an
    /// older query from overwriting results from a newer one.
    search_generation: Arc<AtomicU64>,
    /// Records the path and wall-clock instant of the most recent self-initiated
    /// file save, used to suppress the file-watcher reload that bounces back
    /// within ~700 ms of our own write.
    pub last_file_save_at: Option<(PathBuf, std::time::Instant)>,
    /// Application-level status message shown in the editor footer or status bar.
    pub status_message: Option<String>,
    /// When set, the first file load whose path equals the stored path will
    /// position `cursor_line` at the logical line corresponding to the given
    /// 0-indexed source line, then clear this state.
    ///
    /// Set by [`open_or_focus`] when called from [`confirm_search`] with a
    /// non-`None` jump target.  Consumed (and cleared) in [`apply_file_loaded`].
    pub pending_jump: Option<(PathBuf, u32)>,
    /// When the user passes a file path on the command line, we store it here
    /// so [`run`] can open it once `action_tx` is wired up.  `None` when the
    /// CLI path was a directory (the normal case).
    pub initial_file: Option<PathBuf>,
}

impl App {
    /// Construct a new `App` rooted at `root`.
    ///
    /// Loads persisted config and session state, then auto-restores the last
    /// open file if it still exists on disk.
    ///
    /// # Arguments
    ///
    /// * `root`         – directory used as the tree root.
    /// * `initial_file` – when the user passes a *file* path on the CLI, that
    ///   path is stored here and opened at the start of [`run`] once `action_tx`
    ///   is available.  Pass `None` when the CLI argument is a directory.
    pub fn new(root: PathBuf, initial_file: Option<PathBuf>) -> Self {
        let config = Config::load();
        let palette = Palette::from_theme(config.theme);
        let app_state = AppState::load();

        let entries = FileEntry::discover(&root);
        let mut tree = FileTreeState::default();
        tree.rebuild(entries);
        // Git status is populated asynchronously via `refresh_git_status` once the
        // event loop starts (so action_tx is available).  Starting with an empty map
        // means the tree renders immediately without blocking on `git` subprocess I/O.

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
            search_preview: config.search_preview,
            copy_menu: None,
            app_state,
            action_tx: None,
            pending_chord: None,
            tab_bar_rects: Vec::new(),
            tab_close_rects: Vec::new(),
            tab_picker_rects: Vec::new(),
            search_result_rects: Vec::new(),
            tab_picker: None,
            link_picker: None,
            tree_area_rect: None,
            viewer_area_rect: None,
            mermaid_cache: MermaidCache::new(),
            picker,
            table_modal: None,
            table_modal_rect: None,
            search_generation: Arc::new(AtomicU64::new(0)),
            last_file_save_at: None,
            status_message: None,
            pending_jump: None,
            initial_file,
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
                tab.view
                    .load(path.clone(), name, content, &self.palette, self.theme);
                let max_scroll = tab.view.total_lines.saturating_sub(1);
                let clamped = scroll.min(max_scroll);
                tab.view.scroll_offset = clamped;
                tab.view.cursor_line = clamped;
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
    ///
    /// Delegates to [`FileTreeState::reveal_path`], which handles the ancestor
    /// walk, flat-list rebuild, and selection update in one step.
    fn expand_and_select(&mut self, file: &Path) {
        self.tree.reveal_path(file);
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
            search_preview: self.search_preview,
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

        // Populate git status on a background thread now that action_tx is set.
        // This avoids blocking `App::new` (which runs on the tokio thread) on a
        // potentially slow `git status` subprocess call.
        self.refresh_git_status();

        // If the user passed a file path on the CLI, open it now that action_tx
        // is wired up (open_or_focus spawns a background read that requires it).
        // reveal_path selects the file in the tree so it isn't left blank.
        if let Some(file) = self.initial_file.take() {
            self.expand_and_select(&file);
            self.open_or_focus(file, true, None);
        }

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
                    tab.view.cursor_up(n as u32);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            Action::ScrollDown(n) => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(n as u32);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            Action::ScrollHalfPageUp => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_up(vh / 2);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            Action::ScrollHalfPageDown => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(vh / 2);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            Action::ScrollToTop => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_to_top();
                }
            }
            Action::ScrollToBottom => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_to_bottom(vh);
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
                self.reload_changed_tabs(&changed);
                self.refresh_git_status();
                if let Some(tx) = self.action_tx.clone() {
                    let root = self.root.clone();
                    tokio::task::spawn_blocking(move || {
                        let entries = FileEntry::discover(&root);
                        let _ = tx.send(Action::TreeDiscovered(entries));
                    });
                }
            }
            Action::TreeDiscovered(entries) => {
                self.tree.rebuild(entries);
            }
            Action::Resize(_, _) => {}
            Action::Mouse(m) => self.handle_mouse(m),
            Action::MermaidReady(id, entry) => {
                self.mermaid_cache.insert(id, *entry);
            }
            Action::SearchResults {
                generation,
                results,
                truncated,
            } => {
                // Discard if a newer search has already been started.
                if self.search_generation.load(Ordering::Relaxed) == generation {
                    self.search.results = results;
                    self.search.selected_index = 0;
                    self.search.truncated_at_cap = truncated;
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
            Action::FileSaved {
                path,
                saved_content,
            } => {
                self.apply_file_saved(path, saved_content);
            }
            Action::FileSaveError { path: _, error } => {
                // For the spike: surface in the editor footer if it's open,
                // otherwise fall back to the app-level status message.
                let msg = format!("save error: {error}");
                if let Some(tab) = self.tabs.active_tab_mut()
                    && let Some(editor) = tab.editor.as_mut()
                {
                    editor.status_message = Some(msg);
                } else {
                    self.status_message = Some(msg);
                }
            }
            Action::FileLoadFailed { path } => {
                // Clear a pending_jump that can never be satisfied because the
                // file read failed.  Only clear when the path matches so we
                // don't clobber a pending jump registered for a different file.
                if let Some((ref pending_path, _)) = self.pending_jump
                    && *pending_path == path
                {
                    self.pending_jump = None;
                }
            }
        }
    }

    fn handle_mouse(&mut self, m: crossterm::event::MouseEvent) {
        // The table modal captures all mouse input while it is open.
        // Nothing beneath the modal should react to pointer events.
        if self.table_modal.is_some() {
            self.handle_table_modal_mouse(m);
            return;
        }

        // While the editor is active, mouse events are ignored entirely.
        // The user must `:q` first to exit edit mode before interacting with
        // the tree, tabs, or other panels via pointer.
        if self.focus == Focus::Editor {
            return;
        }

        let col = m.column;
        let row = m.row;

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Search modal rows take priority when the modal is open.
                if self.search.active {
                    let search_hit = self
                        .search_result_rects
                        .iter()
                        .find(|(_, rect)| contains(*rect, col, row))
                        .map(|(idx, _)| *idx);
                    if let Some(idx) = search_hit {
                        self.search.selected_index = idx;
                        self.confirm_search();
                        return;
                    }
                    // Click outside the search modal dismisses it.
                    return;
                }

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

                // Tab close button click (× on each tab).
                let close_hit = self
                    .tab_close_rects
                    .iter()
                    .find(|(_, rect)| contains(*rect, col, row))
                    .map(|(id, _)| *id);

                if let Some(id) = close_hit {
                    self.tabs.close(id);
                    if self.tabs.is_empty() {
                        self.focus = Focus::Tree;
                    }
                    return;
                }

                // Tab bar click (activate).
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
                        tab.view.cursor_down(3);
                        tab.view.scroll_to_cursor(vh);
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
                        tab.view.cursor_up(3);
                        tab.view.scroll_to_cursor(vh);
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
                    // Set the cursor to the heading line itself, then scroll
                    // so 2 lines of context appear above it.
                    tab.view.cursor_line = line.min(tab.view.total_lines.saturating_sub(1));
                    let max = tab.view.total_lines.saturating_sub(vh / 2);
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
            // Editor focus: key events carry the full KeyEvent; reconstruct it
            // from code + modifiers so we can forward to edtui.
            Focus::Editor => {
                let key = crossterm::event::KeyEvent::new(code, modifiers);
                self.handle_editor_key(key);
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
        // Section offsets (cumulative row indices):
        // [0, theme_count)      → Theme
        // [markdown_start]      → Markdown: show_line_numbers
        // [panels_start]        → Panels: tree_position left
        // [panels_start + 1]    → Panels: tree_position right
        // [search_start]        → Search: full_line preview
        // [search_start + 1]    → Search: snippet preview
        const MARKDOWN_ROWS: usize = 1; // "Show line numbers"
        const PANELS_ROWS: usize = 2; // "Tree left", "Tree right"
        let markdown_start = theme_count;
        let panels_start = markdown_start + MARKDOWN_ROWS;
        let search_start = panels_start + PANELS_ROWS;

        if cursor < theme_count {
            let theme = Theme::ALL[cursor];
            self.theme = theme;
            self.palette = Palette::from_theme(theme);
            self.rerender_all_tabs();
            self.persist_config();
        } else if cursor == markdown_start {
            self.show_line_numbers = !self.show_line_numbers;
            self.persist_config();
        } else if cursor == panels_start {
            // Panels: tree left
            self.tree_position = TreePosition::Left;
            self.persist_config();
        } else if cursor == panels_start + 1 {
            // Panels: tree right
            self.tree_position = TreePosition::Right;
            self.persist_config();
        } else if cursor == search_start {
            // Search: full line preview
            self.search_preview = SearchPreview::FullLine;
            self.persist_config();
        } else if cursor == search_start + 1 {
            // Search: snippet preview
            self.search_preview = SearchPreview::Snippet;
            self.persist_config();
        }
    }

    /// Re-render every open tab with the active palette, preserving scroll offsets.
    fn rerender_all_tabs(&mut self) {
        let palette = self.palette;
        self.tabs.rerender_all(&palette, self.theme);
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
            self.table_modal_rect = None;
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
                    tab.view.cursor_to_top();
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

    /// Resolve the second key of a pending `y` chord in the viewer.
    ///
    /// `yy` yanks the current line; any other key cancels the chord.
    /// Returns `true` when the chord was consumed (the caller should return).
    fn resolve_y_chord_viewer(&mut self, code: KeyCode) -> bool {
        if code == KeyCode::Char('y') {
            self.yank_current_line();
            return true;
        }
        false
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

    // ── Editor mode ──────────────────────────────────────────────────────────

    /// Enter vim-style edit mode for the currently active tab.
    ///
    /// Requires the tab to have a `current_path` set (i.e., it was loaded from
    /// disk).  Initialises a [`TabEditor`] from the tab's current rendered source
    /// and switches focus to [`Focus::Editor`].
    ///
    /// The editor starts in Normal mode (matching vim's default).  The user must
    /// press `i` inside the editor to begin inserting text.
    pub fn enter_edit_mode(&mut self) {
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        // Only enter edit mode when we have a real path on disk.
        if tab.view.current_path.is_none() {
            return;
        }
        let content = tab.view.content.clone();
        // Map the viewer cursor's rendered logical line to the exact source line
        // using the block metadata stored at render time.  This is precise: code
        // block borders and table borders are mapped to their fence / header line
        // rather than being offset by visual border rows.
        let target_source_line =
            crate::markdown::source_line_at(&tab.view.rendered, tab.view.cursor_line);
        let source_lines_total = content.split('\n').count();
        let target_row = (target_source_line as usize).min(source_lines_total.saturating_sub(1));
        let mut editor = TabEditor::new(content);
        editor.state.cursor = edtui::Index2::new(target_row, 0);
        tab.editor = Some(editor);
        self.focus = Focus::Editor;
    }

    /// Handle a key event while [`Focus::Editor`] is active.
    ///
    /// Two sub-modes:
    /// - **Command-line mode** (`editor.command_line.is_some()`): we capture chars
    ///   ourselves to build an ex command (`:w`, `:q`, etc.).
    /// - **Editing mode**: forward to edtui, but intercept `:` when edtui is in
    ///   Normal mode to start command-line capture.
    fn handle_editor_key(&mut self, key: crossterm::event::KeyEvent) {
        // We need mutable access to both the tab's editor and `self` (for save
        // dispatch), so extract what we need up front.
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let Some(editor) = tab.editor.as_mut() else {
            // Editor was unexpectedly None; snap back to Viewer.
            self.focus = Focus::Viewer;
            return;
        };

        if editor.command_line.is_some() {
            // ── Command-line capture mode ────────────────────────────────────
            match key.code {
                KeyCode::Esc => {
                    // Cancel command-line; return to editing.
                    editor.command_line = None;
                    editor.status_message = None;
                }
                KeyCode::Backspace => {
                    if let Some(ref mut cmd) = editor.command_line {
                        cmd.pop();
                    }
                }
                KeyCode::Enter => {
                    // Take the command string and dispatch it.
                    let cmd = editor.command_line.take().unwrap_or_default();
                    editor.status_message = None;
                    let outcome = dispatch_command(editor, &cmd);
                    self.apply_command_outcome(outcome);
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut cmd) = editor.command_line {
                        cmd.push(c);
                    }
                }
                _ => {}
            }
        } else {
            // ── Editing mode ─────────────────────────────────────────────────
            // Intercept `:` only when edtui is in Normal mode so that insert
            // mode still inserts a literal colon (matching vim behaviour).
            if key.code == KeyCode::Char(':') && editor.state.mode == EditorMode::Normal {
                editor.command_line = Some(String::new());
                editor.status_message = None;
                return;
            }
            // Everything else goes to edtui.
            forward_key_to_edtui(key, &mut editor.state);
        }
    }

    /// Act on the outcome of an ex-command dispatch.
    ///
    /// Must be called *after* `dispatch_command` returns.  `self.tabs` is
    /// fully accessible here because we're back in `&mut self` context.
    fn apply_command_outcome(&mut self, outcome: CommandOutcome) {
        match outcome {
            CommandOutcome::Handled => {
                // Nothing to do — `dispatch_command` already set any message.
            }
            CommandOutcome::Save => {
                self.save_editor_content(false);
            }
            CommandOutcome::Close => {
                self.close_editor();
            }
            CommandOutcome::SaveThenClose => {
                self.save_editor_content(true);
            }
        }
    }

    /// Initiate an async write of the active tab's editor buffer to disk.
    ///
    /// Uses an atomic rename via `tempfile` to avoid partial writes.  On
    /// completion, sends [`Action::FileSaved`] or [`Action::FileSaveError`].
    ///
    /// If `close_after_save` is `true`, the editor will be closed in the
    /// `FileSaved` handler (`:wq` behaviour).
    fn save_editor_content(&mut self, close_after_save: bool) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let Some(editor) = tab.editor.as_ref() else {
            return;
        };
        let Some(path) = tab.view.current_path.clone() else {
            return;
        };

        let content = extract_text(&editor.state);
        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        // Clone path before moving into the closure so we can also store it in
        // `last_file_save_at` below.
        let path_for_closure = path.clone();
        tokio::task::spawn_blocking(move || {
            let path = path_for_closure;
            // Create the temp file in the same directory so the rename stays
            // on the same filesystem (required for atomic persist()).
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let result: anyhow::Result<()> = (|| {
                use std::io::Write as _;
                let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
                tmp.write_all(content.as_bytes())?;
                tmp.flush()?;
                tmp.persist(&path)?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    let _ = tx.send(Action::FileSaved {
                        path,
                        saved_content: content,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Action::FileSaveError {
                        path,
                        error: e.to_string(),
                    });
                }
            }
        });

        // Record the save time immediately so the watcher grace window starts
        // before the async task completes (conservative: avoids the race where
        // the watcher fires before the action arrives).
        self.last_file_save_at = Some((path, Instant::now()));

        if close_after_save {
            // Set the typed flag so the FileSaved handler knows to close the
            // editor.  This avoids the sentinel-string anti-pattern.
            if let Some(tab) = self.tabs.active_tab_mut()
                && let Some(editor) = tab.editor.as_mut()
            {
                editor.close_after_save = true;
            }
        }
    }

    /// Drop the editor for the active tab and return to [`Focus::Viewer`].
    fn close_editor(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.editor = None;
        }
        self.focus = Focus::Viewer;
    }

    fn handle_viewer_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Resolve pending vim chords before normal dispatch.
        // The `take()` consumes the stored chord; we check `g` and `y` in order.
        let pending = self.pending_chord.take();
        if pending == Some('g') && self.resolve_g_chord_viewer(code) {
            return;
        }
        if pending == Some('y') && self.resolve_y_chord_viewer(code) {
            return;
        }

        match code {
            KeyCode::Enter => {
                self.try_open_table_modal();
            }
            KeyCode::Esc => {
                // In visual mode Esc exits visual selection first.
                if let Some(tab) = self.tabs.active_tab_mut()
                    && tab.view.visual_mode.is_some()
                {
                    tab.view.visual_mode = None;
                    return;
                }
                if let Some(ds) = self.doc_search_mut() {
                    ds.active = false;
                    ds.query.clear();
                    ds.match_lines.clear();
                }
            }
            // `i` enters vim-style edit mode for the active tab's source file.
            KeyCode::Char('i') => {
                self.enter_edit_mode();
            }
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(1);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_up(1);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::Char('d') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(vh / 2);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::Char('u') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_up(vh / 2);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::PageDown => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(vh);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::PageUp => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_up(vh);
                    tab.view.scroll_to_cursor(vh);
                }
            }
            KeyCode::Char('g') => self.pending_chord = Some('g'),
            KeyCode::Char('G') => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_to_bottom(vh);
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
            // `y` in visual mode yanks the selection and exits; otherwise starts the
            // `yy` chord (second `y` copies the current line).
            KeyCode::Char('y') => {
                if let Some(tab) = self.tabs.active_tab_mut()
                    && tab.view.visual_mode.is_some()
                {
                    // Consume visual mode and yank the selection.
                    self.yank_visual_selection();
                } else {
                    // Begin the `yy` chord; next key is resolved at the top of
                    // this function via `resolve_y_chord_viewer`.
                    self.pending_chord = Some('y');
                }
            }
            // `v` toggles char-wise visual mode.
            KeyCode::Char('v') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    use crate::ui::markdown_view::{VisualMode, VisualRange};
                    if tab.view.visual_mode.as_ref().map(|r| r.mode) == Some(VisualMode::Char) {
                        tab.view.visual_mode = None;
                    } else {
                        let line = tab.view.cursor_line;
                        let col = tab.view.cursor_col;
                        tab.view.visual_mode = Some(VisualRange {
                            mode: VisualMode::Char,
                            anchor_line: line,
                            anchor_col: col,
                            cursor_line: line,
                            cursor_col: col,
                        });
                    }
                }
            }
            // `V` toggles visual-line mode.
            KeyCode::Char('V') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    use crate::ui::markdown_view::{VisualMode, VisualRange};
                    if tab.view.visual_mode.as_ref().map(|r| r.mode) == Some(VisualMode::Line) {
                        tab.view.visual_mode = None;
                    } else {
                        let line = tab.view.cursor_line;
                        tab.view.visual_mode = Some(VisualRange {
                            mode: VisualMode::Line,
                            anchor_line: line,
                            anchor_col: 0,
                            cursor_line: line,
                            cursor_col: 0,
                        });
                    }
                }
            }
            // `h` / Left — move cursor column left (only in viewer focus, not tree).
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_col = tab.view.cursor_col.saturating_sub(1);
                    if let Some(range) = tab.view.visual_mode.as_mut() {
                        range.cursor_col = tab.view.cursor_col;
                    }
                }
            }
            // `l` / Right — move cursor column right, clamped to line width.
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    let max = tab.view.current_line_width().saturating_sub(1);
                    tab.view.cursor_col = (tab.view.cursor_col + 1).min(max);
                    if let Some(range) = tab.view.visual_mode.as_mut() {
                        range.cursor_col = tab.view.cursor_col;
                    }
                }
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
                    let vh = self.tabs.view_height;
                    let tab = self.tabs.active_tab_mut();
                    if let Some(tab) = tab
                        && tab.view.total_lines > 0
                    {
                        let max_line = tab.view.total_lines;
                        tab.view.cursor_line = n.min(max_line) - 1;
                        // Use centered scroll so `:N` jumps feel the same as
                        // search-result opens — both are long-distance jumps.
                        tab.view.scroll_to_cursor_centered(vh);
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

    /// Open `path` in a tab, optionally jumping to a source line after load.
    ///
    /// `new_tab == true` pushes a new tab (or focuses an existing one with the
    /// same path). `new_tab == false` replaces the active tab's content.
    ///
    /// If the file is already open the switch is instantaneous (no I/O).
    /// Otherwise the read is dispatched to a background thread and the result
    /// arrives as [`Action::FileLoaded`].
    ///
    /// # Arguments
    ///
    /// * `jump_to_source` – when `Some(line)`, the viewer cursor will be
    ///   positioned at the rendered logical line that corresponds to the given
    ///   0-indexed source line once the file finishes loading.  If the tab is
    ///   already open (i.e. `Focused` outcome), the jump is applied immediately.
    pub fn open_or_focus(&mut self, path: PathBuf, new_tab: bool, jump_to_source: Option<u32>) {
        if path.is_dir() {
            return;
        }

        // If the tab already exists, activating it requires no disk I/O.
        let (_, outcome) = self.tabs.open_or_focus(&path, new_tab);
        if matches!(outcome, OpenOutcome::Focused) {
            // Apply the jump immediately — no FileLoaded event will fire.
            if let Some(source_line) = jump_to_source {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.find_tab_by_path_mut(&path)
                    && let Some(logical) =
                        crate::markdown::logical_line_at_source(&tab.view.rendered, source_line)
                {
                    tab.view.cursor_line = logical;
                    // Centre the jump target so the user sees surrounding
                    // context rather than landing at the viewport edge.
                    tab.view.scroll_to_cursor_centered(vh);
                }
            }
            self.focus = Focus::Viewer;
            return;
        }

        // Store the jump target so `apply_file_loaded` can pick it up when the
        // background read completes.
        if let Some(source_line) = jump_to_source {
            self.pending_jump = Some((path.clone(), source_line));
        }

        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        tokio::task::spawn_blocking(move || {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let _ = tx.send(Action::FileLoaded {
                        path,
                        content,
                        new_tab,
                    });
                }
                Err(_) => {
                    // Notify the main task so it can clear any pending_jump
                    // registered for this path.  No user-facing message yet.
                    let _ = tx.send(Action::FileLoadFailed { path });
                }
            }
        });

        self.focus = Focus::Viewer;
    }

    /// Apply a completed async file load: populate the tab that was reserved
    /// by [`open_or_focus`].
    ///
    /// After loading, if [`pending_jump`] holds a matching path, the viewer
    /// cursor is positioned at the logical line that corresponds to the stored
    /// 0-indexed source line.
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
            let theme = self.theme;
            tab.view.load(path.clone(), name, content, &palette, theme);
        }

        // Apply a pending jump-to-source-line if one was registered for this path.
        // `take()` atomically clears the field; we restore it if the path doesn't
        // match so a later load of the correct file still picks it up.
        if let Some((pending_path, source_line)) = self.pending_jump.take() {
            if pending_path == path {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.find_tab_by_path_mut(&path)
                    && let Some(logical) =
                        crate::markdown::logical_line_at_source(&tab.view.rendered, source_line)
                {
                    tab.view.cursor_line = logical;
                    // Centre the jump target so the user sees surrounding
                    // context rather than landing at the viewport edge.
                    tab.view.scroll_to_cursor_centered(vh);
                }
            } else {
                // Path doesn't match — put it back for a later load.
                self.pending_jump = Some((pending_path, source_line));
            }
        }

        self.focus = Focus::Viewer;
        self.expand_and_select(&path);
    }

    fn open_selected_file(&mut self, new_tab: bool) {
        let Some(path) = self.tree.selected_path().map(|p| p.to_path_buf()) else {
            return;
        };
        self.open_or_focus(path, new_tab, None);
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

        for tab in self.tabs.iter() {
            let Some(path) = tab.view.current_path.clone() else {
                continue;
            };
            if !changed.contains(&path) {
                continue;
            }

            // Suppress reloads that are the echo of our own save.  The
            // debouncer fires up to 500 ms after the write; we guard 700 ms
            // to include a 200 ms safety margin.
            if let Some((ref saved_path, saved_at)) = self.last_file_save_at
                && saved_path == &path
                && saved_at.elapsed() < std::time::Duration::from_millis(700)
            {
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
    /// Skips the reload entirely when the content is byte-for-byte identical to
    /// what is already displayed — this prevents spurious inotify `IN_ACCESS`
    /// events (fired when we *read* a file, not just when it is *written*) from
    /// resetting the cursor back to line 0.
    ///
    /// For genuine content changes, the cursor and scroll are preserved if the
    /// cursor is still within the new line count (file was edited but not
    /// truncated past where the cursor sat).
    fn apply_file_reloaded(&mut self, path: PathBuf, content: String) {
        let palette = self.palette;
        let theme = self.theme;

        for tab in self.tabs.iter_mut() {
            if tab.view.current_path.as_deref() != Some(&*path) {
                continue;
            }

            // Spurious watcher event: content is identical — skip the reload to
            // avoid resetting cursor_line / scroll_offset to 0.
            if content == tab.view.content {
                continue;
            }

            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let old_cursor = tab.view.cursor_line;
            let old_scroll = tab.view.scroll_offset;
            tab.view
                .load(path.clone(), name, content.clone(), &palette, theme);
            // Restore cursor and scroll if still within the (potentially shorter)
            // new document so the user's reading position is preserved on edits.
            let last_line = tab.view.total_lines.saturating_sub(1);
            tab.view.cursor_line = old_cursor.min(last_line);
            tab.view.scroll_offset = old_scroll.min(last_line);
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

    /// Apply a successful editor save.
    ///
    /// Updates the editor baseline so dirty detection is correct, refreshes
    /// `tab.view.content` with the saved text, and closes the editor if
    /// `close_after_save` was set (`:wq` path).
    fn apply_file_saved(&mut self, path: PathBuf, saved_content: String) {
        let palette = self.palette;
        let theme = self.theme;

        // Find the tab for this path and update its editor baseline + view content.
        for tab in self.tabs.iter_mut() {
            if tab.view.current_path.as_ref() != Some(&path) {
                continue;
            }
            // Update the rendered view so the user sees the new content when
            // they return to viewer mode.
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let scroll = tab.view.scroll_offset;
            tab.view
                .load(path.clone(), name, saved_content.clone(), &palette, theme);
            tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));

            if let Some(editor) = tab.editor.as_mut() {
                editor.baseline = saved_content.clone();
                // Close the editor when the `:wq` path requested it.
                let should_close = editor.close_after_save;
                if should_close {
                    tab.editor = None;
                } else if let Some(ed) = tab.editor.as_mut() {
                    ed.status_message = Some("saved".to_string());
                }
            }
            break;
        }

        // If the editor was closed (`:wq`), switch focus back to viewer.
        if let Some(tab) = self.tabs.active_tab()
            && tab.view.current_path.as_ref() == Some(&path)
            && tab.editor.is_none()
        {
            self.focus = Focus::Viewer;
        }

        self.last_file_save_at = Some((path, Instant::now()));

        // Refresh git status so the file tree recolors to reflect the save
        // (new → modified, or modified → clean if the edit was reverted).
        // The watcher suppression above prevents a FilesChanged action from
        // firing, which is where the refresh normally hooks in.
        self.refresh_git_status();
    }

    // ── Search ───────────────────────────────────────────────────────────────

    fn perform_search(&mut self) {
        // Clear stale results immediately so the UI never shows results from a
        // superseded query while the new background task is running.
        self.search.results.clear();
        self.search.selected_index = 0;
        self.search.truncated_at_cap = false;

        if self.search.query.is_empty() {
            return;
        }

        let query = self.search.query.clone();

        match self.search.mode {
            SearchMode::FileName => {
                // Filename search is O(n) over in-memory data — fast enough to
                // run synchronously on the main thread with no perceptible delay.
                // Uses smartcase: uppercase in query → case-sensitive match.
                let sensitive = smartcase_is_sensitive(&query);
                let query_cmp = if sensitive {
                    query.clone()
                } else {
                    query.to_lowercase()
                };

                // Walk all entries (not just flat_items) so collapsed directories
                // are still searched.
                let all_paths = FileEntry::flat_paths(&self.tree.entries);
                for path in all_paths {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let name_cmp = if sensitive {
                        name.clone()
                    } else {
                        name.to_lowercase()
                    };
                    if name_cmp.contains(query_cmp.as_str()) {
                        self.search.results.push(SearchResult {
                            path,
                            name,
                            match_count: 0,
                            preview: String::new(),
                            first_match_line: None,
                        });
                        if self.search.results.len() >= RESULT_CAP {
                            self.search.truncated_at_cap = true;
                            break;
                        }
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
                let preview_mode = self.search_preview;

                tokio::task::spawn_blocking(move || {
                    // Build the match predicate once — avoids re-lowercasing the
                    // query on every line comparison.
                    let sensitive = smartcase_is_sensitive(&query);
                    // Keep a separate clone for preview building (the predicate
                    // closure moves `query` or `query_lower` into itself).
                    let query_for_preview = query.clone();
                    let query_lower = query.to_lowercase();

                    let matches_line: Box<dyn Fn(&str) -> bool + Send> = if sensitive {
                        Box::new(move |line: &str| line.contains(query.as_str()))
                    } else {
                        Box::new(move |line: &str| {
                            line.to_lowercase().contains(query_lower.as_str())
                        })
                    };

                    let mut results: Vec<SearchResult> = Vec::new();
                    let mut truncated = false;

                    'files: for path in paths {
                        // Bail early if a newer search has already started.
                        if gen_arc.load(Ordering::Relaxed) != generation {
                            return;
                        }
                        let Ok(content) = std::fs::read_to_string(&path) else {
                            continue;
                        };

                        let mut match_count = 0usize;
                        let mut first_match: Option<(usize, String)> = None;

                        for (i, line) in content.lines().enumerate() {
                            if matches_line(line) {
                                match_count += 1;
                                if first_match.is_none() {
                                    // Store i as 0-based; confirm_search passes it
                                    // directly as a source-line coordinate.
                                    first_match = Some((
                                        i,
                                        build_preview(line, &query_for_preview, preview_mode),
                                    ));
                                }
                            }
                        }

                        if match_count > 0 {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let (first_line, preview) = first_match.unwrap_or((0, String::new()));
                            results.push(SearchResult {
                                path,
                                name,
                                match_count,
                                preview,
                                first_match_line: Some(first_line),
                            });
                            if results.len() >= RESULT_CAP {
                                truncated = true;
                                break 'files;
                            }
                        }
                    }

                    // Final check before sending — another keystroke may have
                    // arrived while we were iterating the last file.
                    if gen_arc.load(Ordering::Relaxed) == generation {
                        let _ = tx.send(Action::SearchResults {
                            generation,
                            results,
                            truncated,
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

        // Expand ancestor directories and select the file row in the tree so the
        // panel is aligned with the viewer even when the file was in a collapsed
        // subtree.
        self.tree.reveal_path(&result.path);

        // `first_match_line` is 0-based; pass it directly as a source-line
        // coordinate without adjustment.
        let jump_to_source = result.first_match_line.map(|n| n as u32);

        self.open_or_focus(result.path, true, jump_to_source);
    }

    // ── Yank helpers ─────────────────────────────────────────────────────────

    /// Copy the source-level text of the current cursor line to the system
    /// clipboard via OSC 52.  Invoked by the `yy` chord in the viewer.
    fn yank_current_line(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let target_source =
            crate::markdown::source_line_at(&tab.view.rendered, tab.view.cursor_line);
        // `content` is the raw markdown; we index into its lines.
        let content = tab.view.content.clone();
        if let Some(line) = content.lines().nth(target_source as usize) {
            copy_to_clipboard(line);
        }
    }

    /// Copy the source-level text covered by the current visual-line selection
    /// to the system clipboard, then exit visual mode.  Invoked by `y` in visual mode.
    fn yank_visual_selection(&mut self) {
        use crate::ui::markdown_view::{VisualMode, extract_line_text_range};
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let Some(range) = tab.view.visual_mode else {
            return;
        };
        let text = match range.mode {
            VisualMode::Line => {
                // Line mode: yank whole source lines (existing behaviour).
                let top_source =
                    crate::markdown::source_line_at(&tab.view.rendered, range.top_line());
                let bottom_source =
                    crate::markdown::source_line_at(&tab.view.rendered, range.bottom_line());
                build_yank_text(&tab.view.content, top_source, bottom_source)
            }
            VisualMode::Char => {
                // Char mode: extract rendered text from only the selected columns.
                // Walk each line in [top_line, bottom_line] and extract the column
                // range reported by char_range_on_line.
                let mut parts: Vec<String> = Vec::new();
                let mut block_offset = 0u32;
                let top = range.top_line();
                let bottom = range.bottom_line();
                'blocks: for block in &tab.view.rendered {
                    let height = block.height();
                    let block_end = block_offset + height;
                    if block_end <= top {
                        block_offset = block_end;
                        continue;
                    }
                    if block_offset > bottom {
                        break;
                    }
                    if let crate::markdown::DocBlock::Text { text, .. } = block {
                        for (local_idx, line) in text.lines.iter().enumerate() {
                            let abs = block_offset + local_idx as u32;
                            if abs > bottom {
                                break 'blocks;
                            }
                            // Compute display width of this line from its spans.
                            let line_width: u16 = line
                                .spans
                                .iter()
                                .map(|s| {
                                    unicode_width::UnicodeWidthStr::width(s.content.as_ref())
                                        .min(u16::MAX as usize)
                                        as u16
                                })
                                .fold(0u16, |acc, w| acc.saturating_add(w));
                            if let Some((sc, ec)) = range.char_range_on_line(abs, line_width) {
                                parts.push(extract_line_text_range(line, sc, ec));
                            }
                        }
                    }
                    block_offset = block_end;
                }
                parts.join("\n")
            }
        };
        copy_to_clipboard(&text);
        tab.view.visual_mode = None;
    }

    fn shrink_tree(&mut self) {
        // No-op when the tree is hidden: there is nothing visible to resize.
        if !self.tree_hidden {
            self.tree_width_pct = self.tree_width_pct.saturating_sub(5).max(10);
        }
    }

    fn grow_tree(&mut self) {
        // No-op when the tree is hidden: there is nothing visible to resize.
        if !self.tree_hidden {
            self.tree_width_pct = (self.tree_width_pct + 5).min(80);
        }
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
        // Copy the first match line before dropping the `tab` borrow so we can
        // access `self.tabs.view_height` without a conflicting mutable borrow.
        let first_match = tab.doc_search.match_lines.first().copied();

        if let Some(line) = first_match {
            // Mirror the j/k idiom: set cursor_line then let scroll_to_cursor
            // decide whether the viewport needs to move. Setting scroll_offset
            // directly would strand cursor_line at its old position and break
            // subsequent j/k movement.
            let vh = self.tabs.view_height;
            if let Some(tab) = self.tabs.active_tab_mut() {
                tab.view.cursor_line = line;
                tab.view.scroll_to_cursor(vh);
            }
        }
    }

    /// Advance to the next search match, wrapping around.
    ///
    /// Sets `cursor_line` to the match line and calls `scroll_to_cursor` so
    /// subsequent `j`/`k` presses move from the correct row.
    fn doc_search_next(&mut self) {
        let vh = self.tabs.view_height;
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let ds = &mut tab.doc_search;
        if ds.match_lines.is_empty() {
            return;
        }
        ds.current_match = (ds.current_match + 1) % ds.match_lines.len();
        let line = ds.match_lines[ds.current_match];
        tab.view.cursor_line = line;
        tab.view.scroll_to_cursor(vh);
    }

    /// Retreat to the previous search match, wrapping around.
    ///
    /// Sets `cursor_line` to the match line and calls `scroll_to_cursor` so
    /// subsequent `j`/`k` presses move from the correct row.
    fn doc_search_prev(&mut self) {
        let vh = self.tabs.view_height;
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
        tab.view.cursor_line = line;
        tab.view.scroll_to_cursor(vh);
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
        let cursor_line = tab.view.cursor_line;

        // Prefer the table the cursor is currently inside.  Fall back to the
        // first table that intersects the viewport when the cursor is on
        // prose — this preserves the old click-anywhere-to-expand behaviour
        // for files where the cursor hasn't been moved into a table yet.
        let mut cursor_match: Option<&crate::markdown::TableBlock> = None;
        let mut viewport_match: Option<&crate::markdown::TableBlock> = None;
        let mut block_start = 0u32;
        for doc_block in &tab.view.rendered {
            let block_end = block_start + doc_block.height();
            if let crate::markdown::DocBlock::Table(table) = doc_block {
                if cursor_line >= block_start && cursor_line < block_end {
                    cursor_match = Some(table);
                    break;
                }
                if viewport_match.is_none()
                    && block_end > viewport_start
                    && block_start < viewport_end
                {
                    viewport_match = Some(table);
                }
            }
            block_start = block_end;
            if block_start >= viewport_end && cursor_match.is_none() {
                // No more blocks can intersect the viewport; only keep
                // scanning if we still need to find a cursor match.
                if cursor_line < block_start {
                    break;
                }
            }
        }

        let Some(table) = cursor_match.or(viewport_match) else {
            return;
        };
        self.table_modal = Some(TableModalState {
            tab_id: tab.id,
            h_scroll: 0,
            v_scroll: 0,
            headers: table.headers.clone(),
            rows: table.rows.clone(),
            alignments: table.alignments.clone(),
            natural_widths: table.natural_widths.clone(),
        });
        self.focus = Focus::TableModal;
    }

    /// Handle a mouse event while the table modal is open.
    ///
    /// The modal "owns" all mouse input — events that land outside the cached
    /// `table_modal_rect` are silently consumed (so the viewer underneath never
    /// scrolls while the modal is visible).
    ///
    /// Supported gestures:
    /// - Scroll wheel (vertical) inside the modal rect → scroll 3 rows per tick.
    /// - `Shift` + scroll wheel → snap horizontal scroll to the prev/next column
    ///   boundary.
    /// - `ScrollLeft` / `ScrollRight` (trackpad horizontal swipe) → same as
    ///   Shift-scroll-wheel.
    /// - Left-click **outside** the modal rect → close the modal.
    /// - Left-click **inside** the modal rect → no-op (future: cell selection).
    /// - All other events → silently ignored.
    fn handle_table_modal_mouse(&mut self, m: crossterm::event::MouseEvent) {
        use crate::ui::table_modal::{max_h_scroll, next_col_boundary, prev_col_boundary};
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        let col = m.column;
        let row = m.row;
        let inside = self
            .table_modal_rect
            .map(|r| contains(r, col, row))
            // If the rect hasn't been populated yet (first frame), treat the
            // event as inside so we don't inadvertently close on the first click.
            .unwrap_or(true);

        // view_height is used by max_h_scroll to determine the visible horizontal
        // extent; we reuse the viewer's stored height as an approximation.
        let view_height = self.tabs.view_height as u16;

        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    self.close_table_modal();
                }
                // Click inside the modal is a no-op for now.
            }
            MouseEventKind::ScrollDown => {
                if !inside {
                    return;
                }
                if m.modifiers.contains(KeyModifiers::SHIFT) {
                    // Shift + scroll down → advance to next column boundary.
                    if let Some(s) = self.table_modal.as_mut() {
                        let max = max_h_scroll(s, view_height);
                        s.h_scroll = next_col_boundary(&s.natural_widths, s.h_scroll, max);
                    }
                } else if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_add(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if !inside {
                    return;
                }
                if m.modifiers.contains(KeyModifiers::SHIFT) {
                    // Shift + scroll up → retreat to previous column boundary.
                    if let Some(s) = self.table_modal.as_mut() {
                        s.h_scroll = prev_col_boundary(&s.natural_widths, s.h_scroll);
                    }
                } else if let Some(s) = self.table_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(3);
                }
            }
            // Horizontal trackpad gestures (not emitted by all terminals).
            MouseEventKind::ScrollRight => {
                if inside && let Some(s) = self.table_modal.as_mut() {
                    let max = max_h_scroll(s, view_height);
                    s.h_scroll = next_col_boundary(&s.natural_widths, s.h_scroll, max);
                }
            }
            MouseEventKind::ScrollLeft => {
                if inside && let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = prev_col_boundary(&s.natural_widths, s.h_scroll);
                }
            }
            _ => {}
        }
    }

    /// Handle a key press while the table modal is focused.
    ///
    /// Horizontal navigation snaps to column boundaries rather than advancing one
    /// display cell at a time:
    ///
    /// - `h` / `Left`  — jump to the start of the previous column
    /// - `l` / `Right` — jump to the start of the next column
    /// - `H`           — pan left by half the modal inner width
    /// - `L`           — pan right by half the modal inner width
    /// - `0` / `$`     — jump to the leftmost / rightmost position
    /// - `j`/`k`/`d`/`u`/`g`/`G` — vertical navigation (unchanged)
    /// - `q` / `Esc` / `Enter` — close the modal
    fn handle_table_modal_key(&mut self, code: KeyCode) {
        use crate::ui::table_modal::{max_h_scroll, next_col_boundary, prev_col_boundary};

        if self.pending_chord.take() == Some('g') && code == KeyCode::Char('g') {
            if let Some(s) = self.table_modal.as_mut() {
                s.v_scroll = 0;
                s.h_scroll = 0;
            }
            return;
        }

        let view_height = self.tabs.view_height as u16;
        // Derive inner width from the cached modal rect (border is 1 cell on each side).
        // Falls back to 80 before the first draw or in tests that don't call draw.
        let inner_width = self
            .table_modal_rect
            .map(|r| r.width.saturating_sub(2))
            .unwrap_or(80);

        match code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => {
                self.close_table_modal();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = prev_col_boundary(&s.natural_widths, s.h_scroll);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(s) = self.table_modal.as_mut() {
                    let max = max_h_scroll(s, view_height);
                    s.h_scroll = next_col_boundary(&s.natural_widths, s.h_scroll, max);
                }
            }
            KeyCode::Char('H') => {
                if let Some(s) = self.table_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(inner_width / 2);
                }
            }
            KeyCode::Char('L') => {
                if let Some(s) = self.table_modal.as_mut() {
                    let max = max_h_scroll(s, view_height);
                    s.h_scroll = s.h_scroll.saturating_add(inner_width / 2).min(max);
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
    // `MouseEvent` is not pulled in by `use super::*`; the others (KeyModifiers,
    // MouseButton, MouseEventKind) are already in scope from the parent module.
    use crossterm::event::MouseEvent;
    use ratatui::text::{Line, Span, Text};
    use std::cell::Cell;

    fn make_text_block(lines: &[&str]) -> DocBlock {
        let text_lines: Vec<Line<'static>> = lines
            .iter()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        let n = text_lines.len();
        DocBlock::Text {
            text: Text::from(text_lines),
            links: Vec::new(),
            heading_anchors: Vec::new(),
            source_lines: (0..n as u32).collect(),
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
        // Stub row_source_lines: header at line 0, body rows at 2, 3, ...
        let row_source_lines: Vec<u32> = std::iter::once(0)
            .chain((2..).take(rows.len()).map(|i| i as u32))
            .collect();
        DocBlock::Table(TableBlock {
            id: TableBlockId(id),
            headers: h,
            rows: r,
            alignments: vec![pulldown_cmark::Alignment::None; num_cols],
            natural_widths,
            rendered_height: 4,
            source_line: 0,
            row_source_lines,
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
                source_line: 0,
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
            source_line: 0,
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
            source_line: 0,
        }];
        let layouts = HashMap::new();
        let cache = empty_mermaid_cache();
        let result = collect_match_lines(&blocks, &layouts, &cache, "match_me");
        assert_eq!(result, vec![1]);
    }

    // ── table modal key / mouse handler tests ───────────────────────────────

    /// Build an `App` with an active `TableModalState` using the given column
    /// widths and initial scroll positions.  Uses `"."` as the root so it runs
    /// without a special directory.
    fn make_app_with_modal(natural_widths: Vec<usize>, h_scroll: u16, v_scroll: u16) -> App {
        let mut app = App::new(std::path::PathBuf::from("."), None);
        app.table_modal = Some(TableModalState {
            tab_id: crate::ui::tabs::TabId(0),
            h_scroll,
            v_scroll,
            headers: vec![],
            rows: vec![],
            alignments: vec![],
            natural_widths,
        });
        app.focus = Focus::TableModal;
        app
    }

    #[test]
    fn h_key_snaps_to_prev_column_boundary() {
        // widths [10, 20, 15] → boundaries [0, 13, 36]
        // From 17 (inside col 1 which starts at 13), h snaps back to 13.
        let mut app = make_app_with_modal(vec![10, 20, 15], 17, 0);
        app.handle_table_modal_key(KeyCode::Char('h'));
        assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
    }

    #[test]
    fn l_key_snaps_to_next_column_boundary() {
        // From 0, next boundary is 13 (start of col 1).
        let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
        app.handle_table_modal_key(KeyCode::Char('l'));
        assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
    }

    #[test]
    fn capital_h_half_page_left() {
        // inner_width = rect.width - 2 = 42 - 2 = 40; half = 20
        // h_scroll 50 - 20 = 30
        let mut app = make_app_with_modal(vec![10, 20, 15], 50, 0);
        app.table_modal_rect = Some(ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 42,
            height: 20,
        });
        app.handle_table_modal_key(KeyCode::Char('H'));
        assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 30);
    }

    #[test]
    fn scroll_wheel_in_modal_scrolls_vertically() {
        let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
        // Populate the rect so the click registers as "inside".
        app.table_modal_rect = Some(ratatui::layout::Rect {
            x: 5,
            y: 5,
            width: 80,
            height: 30,
        });
        let m = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_table_modal_mouse(m);
        assert_eq!(app.table_modal.as_ref().unwrap().v_scroll, 3);
    }

    #[test]
    fn shift_scroll_in_modal_pans_column() {
        // widths [10, 20, 15] → boundaries [0, 13, 36]; Shift+ScrollDown from 0 → 13
        let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
        app.table_modal_rect = Some(ratatui::layout::Rect {
            x: 5,
            y: 5,
            width: 80,
            height: 30,
        });
        let m = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::SHIFT,
        };
        app.handle_table_modal_mouse(m);
        assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
    }

    #[test]
    fn click_outside_modal_closes_it() {
        let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
        app.table_modal_rect = Some(ratatui::layout::Rect {
            x: 10,
            y: 10,
            width: 60,
            height: 20,
        });
        // Click at (5, 5) — outside the rect (which starts at (10, 10)).
        let m = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_table_modal_mouse(m);
        assert!(
            app.table_modal.is_none(),
            "modal should close on outside click"
        );
    }

    #[test]
    fn click_inside_modal_does_not_close_it() {
        let mut app = make_app_with_modal(vec![10, 20, 15], 5, 2);
        app.table_modal_rect = Some(ratatui::layout::Rect {
            x: 10,
            y: 10,
            width: 60,
            height: 20,
        });
        // Click at (15, 15) — inside the rect.
        let m = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 15,
            row: 15,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_table_modal_mouse(m);
        assert!(
            app.table_modal.is_some(),
            "modal should stay open on inside click"
        );
        // Scroll must not have changed.
        let s = app.table_modal.as_ref().unwrap();
        assert_eq!(s.h_scroll, 5);
        assert_eq!(s.v_scroll, 2);
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

    // ── Editor spike tests ────────────────────────────────────────────────────

    /// Open a tab with known content and put the app in a state suitable for
    /// editor tests.  Returns the `App` and the path used.
    fn make_app_with_tab(content: &str) -> (App, PathBuf) {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/test.md");
        // Use open_or_focus to create the tab, then manually set content.
        app.tabs.open_or_focus(&path, true);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.content = content.to_string();
            tab.view.current_path = Some(path.clone());
            tab.view.file_name = "test.md".to_string();
        }
        app.focus = Focus::Viewer;
        (app, path)
    }

    #[test]
    fn enter_edit_mode_initializes_editor_from_view_content() {
        let (mut app, _path) = make_app_with_tab("# Hello\n\nworld");
        app.enter_edit_mode();
        let tab = app.tabs.active_tab().expect("tab must exist");
        let editor = tab
            .editor
            .as_ref()
            .expect("editor must be Some after enter_edit_mode");
        assert_eq!(editor.baseline, "# Hello\n\nworld");
        assert!(!editor.is_dirty());
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn q_with_no_dirty_returns_to_viewer() {
        let (mut app, _path) = make_app_with_tab("clean content");
        app.enter_edit_mode();
        // Dispatch :q — buffer is clean so the editor should close.
        {
            let tab = app.tabs.active_tab_mut().unwrap();
            let editor = tab.editor.as_mut().unwrap();
            let outcome = dispatch_command(editor, "q");
            // Manually apply the outcome as App::apply_command_outcome would.
            assert_eq!(outcome, CommandOutcome::Close);
        }
        // Simulate the close path.
        app.close_editor();
        assert!(app.tabs.active_tab().unwrap().editor.is_none());
        assert_eq!(app.focus, Focus::Viewer);
    }

    #[test]
    fn q_with_dirty_blocks_and_sets_status_message() {
        let (mut app, _path) = make_app_with_tab("original");
        app.enter_edit_mode();
        // Make it dirty by changing the baseline so the buffer no longer matches.
        {
            let tab = app.tabs.active_tab_mut().unwrap();
            let editor = tab.editor.as_mut().unwrap();
            editor.baseline = "something different".to_string();
            let outcome = dispatch_command(editor, "q");
            assert_eq!(
                outcome,
                CommandOutcome::Handled,
                ":q on dirty buffer must return Handled (not Close)"
            );
            assert!(
                editor.status_message.is_some(),
                "a status message must be set when :q is blocked"
            );
        }
        // Editor must remain open.
        assert!(app.tabs.active_tab().unwrap().editor.is_some());
    }

    #[test]
    fn q_bang_with_dirty_discards_and_returns_to_viewer() {
        let (mut app, _path) = make_app_with_tab("original");
        app.enter_edit_mode();
        {
            let tab = app.tabs.active_tab_mut().unwrap();
            let editor = tab.editor.as_mut().unwrap();
            editor.baseline = "something different".to_string();
            let outcome = dispatch_command(editor, "q!");
            assert_eq!(
                outcome,
                CommandOutcome::Close,
                ":q! must always close even when dirty"
            );
        }
        app.close_editor();
        assert!(app.tabs.active_tab().unwrap().editor.is_none());
        assert_eq!(app.focus, Focus::Viewer);
    }

    #[test]
    fn command_line_captures_chars_until_enter() {
        use crossterm::event::{KeyCode as KC, KeyEvent, KeyModifiers};

        let (mut app, _path) = make_app_with_tab("text");
        app.enter_edit_mode();
        app.focus = Focus::Editor;

        // Press `:` — should start command-line mode (editor is in Normal mode).
        app.handle_editor_key(KeyEvent::new(KC::Char(':'), KeyModifiers::NONE));
        {
            let tab = app.tabs.active_tab().unwrap();
            let editor = tab.editor.as_ref().unwrap();
            assert!(
                editor.command_line.is_some(),
                "':' in Normal mode must start command-line capture"
            );
            assert_eq!(editor.command_line.as_deref(), Some(""));
        }

        // Type 'w'.
        app.handle_editor_key(KeyEvent::new(KC::Char('w'), KeyModifiers::NONE));
        {
            let tab = app.tabs.active_tab().unwrap();
            let editor = tab.editor.as_ref().unwrap();
            assert_eq!(editor.command_line.as_deref(), Some("w"));
        }

        // We can't easily test the Enter path here without an action_tx, so
        // just verify the capture works: 'w' was collected into command_line.
    }

    #[test]
    fn mouse_events_ignored_while_editing() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        let (mut app, _path) = make_app_with_tab("content");
        app.enter_edit_mode();
        // Precondition: focus must be Editor.
        assert_eq!(app.focus, Focus::Editor);

        // Record the tree selection before the mouse event.
        let selection_before = app.tree.list_state.selected();

        // Simulate a left-click anywhere on screen.
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(click);

        // Focus must remain on the editor.
        assert_eq!(app.focus, Focus::Editor, "focus must stay Editor");
        // Tree selection must be unchanged.
        assert_eq!(
            app.tree.list_state.selected(),
            selection_before,
            "tree selection must not change during edit mode"
        );
        // Editor must still be present.
        assert!(
            app.tabs.active_tab().unwrap().editor.is_some(),
            "editor must remain open"
        );
    }

    // ── enter_edit_mode source-line tests ────────────────────────────────────

    /// `enter_edit_mode` must place the edtui cursor on the source line that
    /// the viewer cursor's rendered logical line maps to via `source_line_at`.
    ///
    /// We build a Text block whose `source_lines` are [10, 11, 12] and set the
    /// viewer cursor to logical line 1.  `source_line_at` returns 11, so the
    /// editor cursor row must be 11.
    #[test]
    fn enter_edit_mode_uses_cursor_for_source_line() {
        use crate::markdown::{DocBlock, HeadingAnchor, LinkInfo};
        use ratatui::text::{Line, Span, Text};

        let mut app = App::new(std::path::PathBuf::from("."), None);

        // Open a tab with dummy content that has as many newlines as the
        // highest source line we reference (line 11 → 12 lines).
        let content: String = (0..12).map(|i| format!("source line {i}\n")).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        std::fs::write(&path, &content).unwrap();

        let (_, _) = app.tabs.open_or_focus(&path, true);
        let palette = crate::theme::Palette::from_theme(crate::theme::Theme::Default);
        let tab = app.tabs.active_tab_mut().unwrap();
        tab.view.load(
            path.clone(),
            "test.md".into(),
            content,
            &palette,
            crate::theme::Theme::Default,
        );

        // Replace the rendered blocks with a hand-crafted Text block whose
        // source_lines are [10, 11, 12].
        let src_lines = vec![10u32, 11, 12];
        let text_lines: Vec<Line<'static>> = src_lines
            .iter()
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect();
        tab.view.rendered = vec![DocBlock::Text {
            text: Text::from(text_lines),
            links: Vec::<LinkInfo>::new(),
            heading_anchors: Vec::<HeadingAnchor>::new(),
            source_lines: src_lines,
        }];
        tab.view.total_lines = 3;
        // Set cursor to logical line 1 → source_line_at returns 11.
        tab.view.cursor_line = 1;

        app.focus = Focus::Viewer;
        app.enter_edit_mode();

        assert_eq!(app.focus, Focus::Editor, "focus should switch to Editor");
        let tab = app.tabs.active_tab().unwrap();
        let editor = tab.editor.as_ref().expect("editor should be set");
        assert_eq!(
            editor.state.cursor.row, 11,
            "editor cursor row should be the mapped source line (11)"
        );
    }

    // ── viewer navigation (d/u/gg/G) regression tests ────────────────────────

    /// Minimal App with a tab whose view has a known `total_lines` and a
    /// configured `view_height`.  Cheaper than `make_app_with_tab` because it
    /// does not load + render real markdown content.
    fn make_app_with_view(total_lines: u32, view_height: u32) -> App {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/nav_test.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = view_height;
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.total_lines = total_lines;
            tab.view.cursor_line = 0;
            tab.view.scroll_offset = 0;
        }
        app.focus = Focus::Viewer;
        app
    }

    #[test]
    fn d_key_moves_cursor_half_page_down() {
        let mut app = make_app_with_view(100, 30);
        app.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, 15,
            "`d` should move the cursor half a page (vh/2 = 15)"
        );
    }

    #[test]
    fn u_key_moves_cursor_half_page_up() {
        let mut app = make_app_with_view(100, 30);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.cursor_line = 50;
            tab.view.scroll_offset = 35;
        }
        app.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.view.cursor_line, 35, "`u` should move cursor up vh/2");
    }

    #[test]
    fn gg_chord_jumps_cursor_to_top() {
        let mut app = make_app_with_view(100, 30);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.cursor_line = 50;
            tab.view.scroll_offset = 35;
        }
        app.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        app.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.view.cursor_line, 0, "`gg` should jump cursor to 0");
        assert_eq!(tab.view.scroll_offset, 0, "`gg` should reset scroll");
    }

    #[test]
    fn shift_g_jumps_cursor_to_bottom() {
        let mut app = make_app_with_view(100, 30);
        app.handle_key(KeyCode::Char('G'), KeyModifiers::SHIFT);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, 99,
            "`G` should land cursor on last line"
        );
    }

    /// When the cursor is inside a table block, `Enter` must open THAT
    /// table rather than the first table visible on screen.
    #[test]
    fn try_open_table_modal_picks_table_under_cursor() {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/tables.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 30;
        app.focus = Focus::Viewer;

        // Layout: [text(3)] [table A(4)] [text(3)] [table B(4)]
        //          0..3      3..7         7..10     10..14
        let blocks = vec![
            make_text_block(&["intro", "text", "here"]),
            make_table_block(10, &["A"], &[&["a-row-0"]]),
            make_text_block(&["middle", "text", "here"]),
            make_table_block(20, &["B"], &[&["b-row-0"]]),
        ];
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.total_lines = blocks.iter().map(|b| b.height()).sum();
            tab.view.rendered = blocks;
            tab.view.scroll_offset = 0;
            tab.view.cursor_line = 12; // inside table B (10..14)
        }

        app.try_open_table_modal();
        let modal = app.table_modal.as_ref().expect("modal must open");
        assert_eq!(
            modal.headers.len(),
            1,
            "expected table B's single header, got {:?}",
            modal.headers
        );
        assert_eq!(
            modal.rows[0][0]
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>(),
            "b-row-0",
            "modal should carry table B's data, not table A's",
        );
    }

    /// Regression: when the cursor is on prose (not a table), `Enter` should
    /// fall back to the first table intersecting the viewport (old behaviour).
    #[test]
    fn try_open_table_modal_falls_back_to_first_visible_table() {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/tables.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 30;
        app.focus = Focus::Viewer;

        let blocks = vec![
            make_text_block(&["intro"]),
            make_table_block(10, &["A"], &[&["a-row-0"]]),
            make_table_block(20, &["B"], &[&["b-row-0"]]),
        ];
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.total_lines = blocks.iter().map(|b| b.height()).sum();
            tab.view.rendered = blocks;
            tab.view.scroll_offset = 0;
            tab.view.cursor_line = 0; // on prose, above any table
        }

        app.try_open_table_modal();
        let modal = app.table_modal.as_ref().expect("modal must open");
        assert_eq!(
            modal.rows[0][0]
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>(),
            "a-row-0",
            "modal should open table A (first visible) when cursor is on prose",
        );
    }

    #[test]
    fn d_key_moves_cursor_with_real_loaded_content() {
        use crate::theme::{Palette, Theme};
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/nav_test.md");
        app.tabs.open_or_focus(&path, true);
        let content: String = (0..60).map(|i| format!("paragraph {i}\n\n")).collect();
        let palette = Palette::from_theme(Theme::Default);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.load(
                path.clone(),
                "nav_test.md".to_string(),
                content,
                &palette,
                Theme::Default,
            );
        }
        app.focus = Focus::Viewer;
        app.tabs.view_height = 30;

        let before_cursor = app.tabs.active_tab().unwrap().view.cursor_line;
        let before_total = app.tabs.active_tab().unwrap().view.total_lines;
        let before_vh = app.tabs.view_height;
        app.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        let after_cursor = app.tabs.active_tab().unwrap().view.cursor_line;
        assert!(
            before_total > 0,
            "total_lines must be populated (got {before_total})"
        );
        assert!(
            before_vh > 0,
            "view_height must be positive (got {before_vh})"
        );
        assert_ne!(
            before_cursor, after_cursor,
            "`d` should move the cursor (before={before_cursor} after={after_cursor} \
             total_lines={before_total} view_height={before_vh})",
        );
    }

    // ── doc_search navigation ────────────────────────────────────────────────

    /// Build an `App` with an active tab whose `doc_search` state has the
    /// given match lines and current_match, and whose view has the given
    /// total_lines.  view_height defaults to 20.
    fn make_app_with_doc_search(
        match_lines: Vec<u32>,
        current_match: usize,
        total_lines: u32,
    ) -> App {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/ds_test.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.total_lines = total_lines;
            tab.view.cursor_line = 0;
            tab.view.scroll_offset = 0;
            tab.doc_search.match_lines = match_lines;
            tab.doc_search.current_match = current_match;
        }
        app
    }

    /// `doc_search_next` must advance `current_match`, set `cursor_line` to the
    /// new match line, and adjust `scroll_offset` via `scroll_to_cursor`.
    #[test]
    fn doc_search_next_updates_cursor_and_scroll() {
        // 100-line doc, view_height = 20; match_lines = [5, 20, 35],
        // cursor starts at line 5 (current_match = 0).
        let mut app = make_app_with_doc_search(vec![5, 20, 35], 0, 100);
        {
            // Ensure cursor is already at the first match.
            let tab = app.tabs.active_tab_mut().unwrap();
            tab.view.cursor_line = 5;
        }
        app.doc_search_next();
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.doc_search.current_match, 1);
        assert_eq!(
            tab.view.cursor_line, 20,
            "cursor must move to match line 20"
        );
        // After scroll_to_cursor with view_height=20, scroll_offset = 20 - (20-1) = 1.
        assert_eq!(tab.view.scroll_offset, 1);
    }

    /// `doc_search_prev` with `current_match == 0` must wrap to the last match.
    #[test]
    fn doc_search_prev_wraps_to_last_match() {
        let mut app = make_app_with_doc_search(vec![5, 20, 35], 0, 100);
        app.doc_search_prev();
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.doc_search.current_match, 2);
        assert_eq!(tab.view.cursor_line, 35, "cursor must wrap to last match");
    }

    /// When there are no matches, `doc_search_next` must not change any state.
    #[test]
    fn doc_search_empty_matches_no_op() {
        let mut app = make_app_with_doc_search(vec![], 0, 100);
        {
            let tab = app.tabs.active_tab_mut().unwrap();
            tab.view.cursor_line = 7;
            tab.view.scroll_offset = 3;
        }
        app.doc_search_next();
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.view.cursor_line, 7, "cursor must not change");
        assert_eq!(tab.view.scroll_offset, 3, "scroll must not change");
    }

    /// `perform_doc_search` with a matching query must set `cursor_line` to the
    /// first match.
    ///
    /// We build rendered blocks that contain "hello" on line 4 (the 5th line
    /// of a Text block that starts at the document root) and verify the cursor
    /// ends up at absolute line 4.
    #[test]
    fn perform_doc_search_first_match_moves_cursor() {
        let lines: Vec<&str> = (0..10)
            .map(|i| if i == 4 { "hello world" } else { "other" })
            .collect();
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/search_test.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        if let Some(tab) = app.tabs.active_tab_mut() {
            let block = make_text_block(lines.as_slice());
            let total = block.height();
            tab.view.rendered = vec![block];
            tab.view.total_lines = total;
            tab.view.cursor_line = 0;
            tab.view.scroll_offset = 0;
            tab.doc_search.active = true;
            tab.doc_search.query = "hello".to_string();
        }
        app.focus = Focus::Viewer;
        app.perform_doc_search();
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, 4,
            "cursor must jump to first match at line 4"
        );
    }

    #[test]
    fn watcher_suppresses_reload_within_grace_window() {
        let (mut app, path) = make_app_with_tab("content");
        // Simulate a recent self-save.
        app.last_file_save_at = Some((path.clone(), Instant::now()));
        // reload_changed_tabs requires action_tx; if None it returns early before
        // the suppression check.  We use a channel so the logic actually runs.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Action>();
        app.action_tx = Some(tx);
        app.reload_changed_tabs(std::slice::from_ref(&path));
        // The spawn_blocking must NOT have been called because the path is
        // within the grace window.  Since spawn_blocking is async, we check that
        // no FileReloaded action arrives immediately (the channel should be empty).
        assert!(
            rx.try_recv().is_err(),
            "no FileReloaded should be sent when within the grace window"
        );
    }

    // ── apply_file_reloaded cursor-preservation ──────────────────────────────

    /// A `FileReloaded` event with unchanged content must not reset the cursor.
    ///
    /// On Linux, inotify fires `IN_ACCESS` when a file is *read*, producing a
    /// spurious `FilesChanged` → `FileReloaded` round-trip.  The guard in
    /// `apply_file_reloaded` compares byte content and skips the reload, so the
    /// cursor stays wherever the user left it.
    #[test]
    fn reload_with_unchanged_content_preserves_cursor() {
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        let content: String = (0..20).map(|i| format!("line {i}\n\n")).collect();
        let path = PathBuf::from("/fake/unchanged.md");

        let mut app = App::new(PathBuf::from("."), None);
        app.tabs.open_or_focus(&path, true);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.load(
                path.clone(),
                "unchanged.md".to_string(),
                content.clone(),
                &palette,
                Theme::Default,
            );
            tab.view.cursor_line = 10;
            tab.view.scroll_offset = 5;
        }

        // Simulate FileReloaded arriving with identical content.
        app.apply_file_reloaded(path.clone(), content);

        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, 10,
            "cursor must not reset on spurious reload (unchanged content)"
        );
        assert_eq!(
            tab.view.scroll_offset, 5,
            "scroll must not reset on spurious reload (unchanged content)"
        );
    }

    /// A `FileReloaded` event with new content must restore the cursor to its
    /// old position when that position is still valid (file grew or same size).
    #[test]
    fn reload_with_changed_content_restores_cursor_when_in_range() {
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        // 20 paragraphs → many display lines.
        let content_v1: String = (0..20).map(|i| format!("line {i}\n\n")).collect();
        let path = PathBuf::from("/fake/changed.md");

        let mut app = App::new(PathBuf::from("."), None);
        app.tabs.open_or_focus(&path, true);
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.load(
                path.clone(),
                "changed.md".to_string(),
                content_v1,
                &palette,
                Theme::Default,
            );
            tab.view.cursor_line = 10;
            tab.view.scroll_offset = 5;
        }

        // New content that is longer than 10 display lines — cursor stays.
        let content_v2: String = (0..20).map(|i| format!("edited {i}\n\n")).collect();
        app.apply_file_reloaded(path.clone(), content_v2);

        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, 10,
            "cursor must be restored after a genuine reload when still in range"
        );
    }

    // ── build_yank_text ──────────────────────────────────────────────────────

    #[test]
    fn build_yank_text_single_line() {
        let content = "alpha\nbeta\ngamma";
        assert_eq!(build_yank_text(content, 1, 1), "beta");
    }

    #[test]
    fn build_yank_text_multi_line() {
        let content = "line0\nline1\nline2\nline3";
        assert_eq!(build_yank_text(content, 1, 3), "line1\nline2\nline3");
    }

    #[test]
    fn build_yank_text_reversed_range() {
        // Range given in reverse order must produce same result as forward range.
        let content = "a\nb\nc";
        assert_eq!(build_yank_text(content, 2, 0), "a\nb\nc");
    }

    #[test]
    fn build_yank_text_past_eof() {
        // Range that extends past the available lines returns whatever is there.
        let content = "x\ny";
        let result = build_yank_text(content, 0, 10);
        assert_eq!(result, "x\ny");
    }

    #[test]
    fn build_yank_text_empty_content() {
        assert_eq!(build_yank_text("", 0, 0), "");
    }

    // ── Feature 2: Visual mode and yank ─────────────────────────────────────

    /// Helper: build an App with a rendered tab (blocks set, not just content string).
    fn make_rendered_app(content: &str) -> (App, PathBuf) {
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        let path = PathBuf::from("/fake/yank_test.md");
        let mut app = App::new(PathBuf::from("."), None);
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.load(
                path.clone(),
                "yank_test.md".to_string(),
                content.to_string(),
                &palette,
                Theme::Default,
            );
        }
        app.focus = Focus::Viewer;
        (app, path)
    }

    /// Helper to build a line-mode `VisualRange` for tests.
    fn line_vrange(anchor: u32, cursor: u32) -> crate::ui::markdown_view::VisualRange {
        use crate::ui::markdown_view::{VisualMode, VisualRange};
        VisualRange {
            mode: VisualMode::Line,
            anchor_line: anchor,
            anchor_col: 0,
            cursor_line: cursor,
            cursor_col: 0,
        }
    }

    #[test]
    fn capital_v_enters_line_visual_mode() {
        let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
        use crate::ui::markdown_view::{VisualMode, VisualRange};
        // Move cursor to line 2.
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.cursor_line = 2;
        }
        app.handle_key(KeyCode::Char('V'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.visual_mode,
            Some(VisualRange {
                mode: VisualMode::Line,
                anchor_line: 2,
                anchor_col: 0,
                cursor_line: 2,
                cursor_col: 0,
            }),
            "V must enter line visual mode at current cursor"
        );
    }

    #[test]
    fn lowercase_v_enters_char_visual_mode() {
        let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
        use crate::ui::markdown_view::{VisualMode, VisualRange};
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.cursor_line = 1;
            tab.view.cursor_col = 3;
        }
        app.handle_key(KeyCode::Char('v'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.visual_mode,
            Some(VisualRange {
                mode: VisualMode::Char,
                anchor_line: 1,
                anchor_col: 3,
                cursor_line: 1,
                cursor_col: 3,
            }),
            "v must enter char visual mode at current cursor/col"
        );
    }

    #[test]
    fn v_in_visual_mode_exits_visual_mode() {
        let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
        // Enter line visual mode manually, then press V again to exit.
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.visual_mode = Some(line_vrange(1, 2));
        }
        app.handle_key(KeyCode::Char('V'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.visual_mode, None,
            "V in line visual mode must exit it"
        );
    }

    #[test]
    fn esc_in_visual_mode_exits_visual_mode() {
        let (mut app, _path) = make_rendered_app("line0\nline1");
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.view.visual_mode = Some(line_vrange(0, 1));
        }
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.view.visual_mode, None, "Esc must exit visual mode");
    }

    #[test]
    fn j_in_visual_mode_extends_range() {
        // Use a controlled tab with known total_lines to avoid renderer side-effects.
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/visual_j.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        if let Some(tab) = app.tabs.active_tab_mut() {
            // Build 10 logical lines directly so the cursor clamp works correctly.
            let block = make_text_block(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
            let total = block.height();
            tab.view.rendered = vec![block];
            tab.view.total_lines = total;
            tab.view.cursor_line = 2;
            tab.view.visual_mode = Some(line_vrange(2, 2));
        }
        app.focus = Focus::Viewer;
        // Press j to move down.
        app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        let range = tab
            .view
            .visual_mode
            .expect("visual mode must still be active");
        assert_eq!(range.anchor_line, 2, "anchor must stay at 2");
        assert_eq!(range.cursor_line, 3, "cursor must extend to 3 after j");
    }

    #[test]
    fn y_in_visual_mode_yanks_and_exits() {
        // Use a controlled tab with predictable source_lines mapping.
        // make_text_block assigns source_lines = [0, 1, 2, ...] sequentially.
        let content = "alpha\nbeta\ngamma\ndelta";
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/visual_yank.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        if let Some(tab) = app.tabs.active_tab_mut() {
            let block = make_text_block(&["alpha", "beta", "gamma", "delta"]);
            let total = block.height();
            tab.view.rendered = vec![block];
            tab.view.total_lines = total;
            tab.view.content = content.to_string();
            tab.view.current_path = Some(path.clone());
            // Select logical lines 1..=2 (source lines 1="beta", 2="gamma").
            tab.view.cursor_line = 1;
            tab.view.visual_mode = Some(line_vrange(1, 2));
        }
        app.focus = Focus::Viewer;
        // Press y — should yank and exit visual mode.
        app.handle_key(KeyCode::Char('y'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.visual_mode, None,
            "y in visual mode must exit visual mode"
        );
        // Verify that the yank text for source lines 1..=2 is correct.
        let top_source = crate::markdown::source_line_at(&tab.view.rendered, 1);
        let bottom_source = crate::markdown::source_line_at(&tab.view.rendered, 2);
        let expected = build_yank_text(content, top_source, bottom_source);
        assert_eq!(
            expected, "beta\ngamma",
            "yank text must span visual selection"
        );
    }

    // ── New: h/l cursor column movement ─────────────────────────────────────

    #[test]
    fn h_moves_cursor_col_left() {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/hl_test.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        // Build a line wide enough to have horizontal room.
        if let Some(tab) = app.tabs.active_tab_mut() {
            let block = make_text_block(&["hello world"]);
            tab.view.rendered = vec![block];
            tab.view.total_lines = 1;
            tab.view.cursor_col = 5;
        }
        app.focus = Focus::Viewer;
        app.handle_key(KeyCode::Char('h'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(tab.view.cursor_col, 4, "h must decrement cursor_col");
    }

    #[test]
    fn l_moves_cursor_col_right_clamped() {
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/hl_clamp.md");
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;
        // "abc" is 3 cells wide — max cursor_col = 2.
        if let Some(tab) = app.tabs.active_tab_mut() {
            let block = make_text_block(&["abc"]);
            tab.view.rendered = vec![block];
            tab.view.total_lines = 1;
            tab.view.cursor_col = 2; // already at end
        }
        app.focus = Focus::Viewer;
        app.handle_key(KeyCode::Char('l'), KeyModifiers::NONE);
        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_col, 2,
            "l at end of line must not exceed line_width-1"
        );
    }

    // ── Feature 1: confirm_search jumps to match line ───────────────────────

    #[test]
    fn pending_jump_cleared_after_apply() {
        // Set a pending jump and simulate a FileLoaded action for the same path.
        let path = PathBuf::from("/fake/jump_test.md");
        let content = "line0\nline1\nline2\nline3\nline4";
        let mut app = App::new(PathBuf::from("."), None);
        app.tabs.open_or_focus(&path, true);
        // Seed the tab as empty (simulates a pending load).
        // pending_jump is set to source line 2.
        app.pending_jump = Some((path.clone(), 2));
        // Now simulate FileLoaded arriving.
        app.apply_file_loaded(path.clone(), content.to_string(), true);
        assert!(
            app.pending_jump.is_none(),
            "pending_jump must be cleared after apply_file_loaded"
        );
    }

    #[test]
    fn confirm_search_filename_result_no_jump() {
        // A filename-mode result has first_match_line = None;
        // after the search confirm, pending_jump should remain None.
        use crate::ui::search_modal::{SearchMode, SearchResult};
        let mut app = App::new(PathBuf::from("."), None);
        let path = PathBuf::from("/fake/fn_result.md");
        app.search.active = true;
        app.search.mode = SearchMode::FileName;
        app.search.results = vec![SearchResult {
            path: path.clone(),
            name: "fn_result.md".to_string(),
            match_count: 0,
            preview: String::new(),
            first_match_line: None,
        }];
        app.search.selected_index = 0;
        app.confirm_search();
        assert!(
            app.pending_jump.is_none(),
            "filename result must not set pending_jump"
        );
    }

    #[test]
    fn apply_file_loaded_jumps_cursor_to_source_line() {
        // Verify that apply_file_loaded applies the pending_jump by setting
        // cursor_line to the logical line that corresponds to a given source line.
        //
        // We use a controlled tab with a direct DocBlock whose source_lines are
        // sequential (0, 1, 2, …), bypassing the markdown renderer so we can
        // predict the mapping exactly.
        let content = "alpha\nbeta\ngamma\ndelta\nepsilon";
        let path = PathBuf::from("/fake/jump_cursor.md");
        let mut app = App::new(PathBuf::from("."), None);
        app.tabs.open_or_focus(&path, true);
        app.tabs.view_height = 20;

        // Populate the tab with a known block (source_lines = [0,1,2,3,4]).
        if let Some(tab) = app.tabs.active_tab_mut() {
            let block = make_text_block(&["alpha", "beta", "gamma", "delta", "epsilon"]);
            let total = block.height();
            tab.view.rendered = vec![block];
            tab.view.total_lines = total;
            tab.view.content = content.to_string();
            tab.view.current_path = Some(path.clone());
        }

        // Confirm that logical_line_at_source maps source line 2 to logical 2
        // in our controlled block.
        let expected_logical = {
            let tab = app.tabs.active_tab().unwrap();
            crate::markdown::logical_line_at_source(&tab.view.rendered, 2)
                .expect("controlled block must map source 2 to logical 2")
        };
        assert_eq!(
            expected_logical, 2,
            "make_text_block must yield source_line == logical_line"
        );

        // Now simulate a pending jump followed by a fresh FileLoaded event.
        // Because content is non-empty, apply_file_loaded won't call load();
        // but the pending_jump logic still runs and moves the cursor.
        app.pending_jump = Some((path.clone(), 2));
        app.apply_file_loaded(path.clone(), content.to_string(), true);

        let tab = app.tabs.active_tab().unwrap();
        assert_eq!(
            tab.view.cursor_line, expected_logical,
            "cursor_line must land on logical line {expected_logical} for source line 2"
        );
        assert!(app.pending_jump.is_none(), "pending_jump must be consumed");
    }

    #[test]
    fn pending_jump_cleared_on_file_load_failure() {
        // A FileLoadFailed for the matching path must clear pending_jump.
        let path = PathBuf::from("/fake/nonexistent.md");
        let mut app = App::new(PathBuf::from("."), None);
        app.pending_jump = Some((path.clone(), 5));
        app.handle_action(Action::FileLoadFailed { path: path.clone() });
        assert!(
            app.pending_jump.is_none(),
            "pending_jump must be cleared when the matching file fails to load"
        );
    }

    #[test]
    fn pending_jump_not_cleared_on_different_path_failure() {
        // A FileLoadFailed for a different path must not touch pending_jump.
        let path = PathBuf::from("/fake/target.md");
        let other = PathBuf::from("/fake/other.md");
        let mut app = App::new(PathBuf::from("."), None);
        app.pending_jump = Some((path.clone(), 3));
        app.handle_action(Action::FileLoadFailed { path: other });
        assert!(
            app.pending_jump.is_some(),
            "pending_jump must be preserved when a different file fails to load"
        );
    }
}
