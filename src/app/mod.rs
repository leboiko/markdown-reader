use crate::action::Action;
use crate::cast::u32_sat;
use crate::config::{Config, SearchPreview, TreePosition};
use crate::event::EventHandler;
use crate::fs::discovery::FileEntry;
use crate::fs::git_status;
use crate::markdown::DocBlock;
use crate::mermaid::{MermaidCache, MermaidEntry};
use crate::state::{AppState, TabSession};
use crate::theme::{Palette, Theme};
use crate::ui::file_tree::FileTreeState;
use crate::ui::link_picker::LinkPickerState;
use crate::ui::markdown_view::TableLayout;
use crate::ui::search_modal::SearchState;
use crate::ui::tab_picker::TabPickerState;
use crate::ui::tabs::{OpenOutcome, Tabs};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

// ── Submodule implementations ────────────────────────────────────────────────
// Each file contains `impl App { ... }` blocks for a logical group of methods.
// They use `use super::*;` to access all types and free functions from this
// module without repeating imports.

mod file_ops;
mod key_handlers;
mod search;
mod table_modal;
mod yank;

// ── Free functions ───────────────────────────────────────────────────────────

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
/// Mermaid: only match when the entry is showing as source (`Failed` / `SourceOnly` /
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
                        matches.push(offset + u32_sat(i));
                    }
                }
                offset += u32_sat(text.lines.len());
            }
            DocBlock::Table(table) => {
                if let Some(layout) = table_layouts.get(&table.id) {
                    for (i, line) in layout.text.lines.iter().enumerate() {
                        let line_text: String =
                            line.spans.iter().map(|s| s.content.as_ref()).collect();
                        if line_text.to_lowercase().contains(query_lower) {
                            matches.push(offset + u32_sat(i));
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
                let show_as_source = match mermaid_cache.get(*id) {
                    None | Some(MermaidEntry::Failed(_) | MermaidEntry::SourceOnly(_)) => {
                        true
                    }
                    Some(MermaidEntry::Pending | MermaidEntry::Ready { .. }) => false,
                };
                if show_as_source {
                    let limit = block_height.saturating_sub(1) as usize;
                    for (i, line) in source.lines().take(limit).enumerate() {
                        if line.to_lowercase().contains(query_lower) {
                            matches.push(offset + u32_sat(i));
                        }
                    }
                }
                offset += block_height;
            }
        }
    }

    matches
}

// ── Focus ────────────────────────────────────────────────────────────────────

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

// ── Ancillary state types ────────────────────────────────────────────────────

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

    /// Total number of rows across all sections.
    pub fn total_rows() -> usize {
        Self::SECTIONS.iter().map(|(_, n)| n).sum()
    }

    /// Move the cursor one row up (wrapping).
    pub fn move_up(&mut self) {
        let total = Self::total_rows();
        self.cursor = (self.cursor + total - 1) % total;
    }

    /// Move the cursor one row down (wrapping).
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

// ── App ──────────────────────────────────────────────────────────────────────

/// Top-level application state.
#[allow(clippy::struct_excessive_bools)]
#[allow(clippy::struct_field_names)]
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
        let Some(session) = self.app_state.sessions.get(&self.root).cloned() else {
            return;
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
                // SAFETY: we just opened or replaced a tab, so active_tab_mut
                // is guaranteed to return Some here.
                let tab = self
                    .tabs
                    .active_tab_mut()
                    .expect("active tab must exist after open_or_focus");
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

    // ── Re-render helpers ────────────────────────────────────────────────────

    /// Re-render every open tab with the active palette, preserving scroll offsets.
    fn rerender_all_tabs(&mut self) {
        let palette = self.palette;
        self.tabs.rerender_all(&palette, self.theme);
        // Mermaid images have the theme background baked into their pixels,
        // so they must re-render when the theme changes.
        self.mermaid_cache.clear();
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

    /// Central action dispatcher: translate every [`Action`] variant into one or
    /// more state mutations.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn handle_action(&mut self, action: Action) {
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
                    tab.view.cursor_up(u32::from(n));
                    tab.view.scroll_to_cursor(vh);
                }
            }
            Action::ScrollDown(n) => {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_down(u32::from(n));
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
                // Surface in the editor footer if it's open, otherwise fall
                // back to the app-level status message.
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

    /// Top-level mouse-event dispatcher.
    #[allow(clippy::too_many_lines)]
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

    /// Top-level key-event dispatcher.
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
}


// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
