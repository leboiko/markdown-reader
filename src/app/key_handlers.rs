/// Key-event handler implementations for every focus mode.
///
/// Each method in this file is part of `impl App`.  They are split from
/// `mod.rs` purely for readability; no new types are introduced and the
/// public surface of `App` is unchanged.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;

impl App {
    // ── Config popup ─────────────────────────────────────────────────────────

    /// Handle a key press while the settings popup ([`Focus::Config`]) is open.
    pub(super) fn handle_config_key(&mut self, code: KeyCode) {
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
                self.focus = if self.tree_hidden && self.pre_config_focus == Focus::Tree {
                    Focus::Viewer
                } else {
                    self.pre_config_focus
                };
            }
            KeyCode::Char('q') => self.running = false,
            _ => {}
        }
    }

    /// Map the flat cursor position to the concrete setting it represents and
    /// toggle / select it.
    pub(super) fn apply_config_selection(&mut self, cursor: usize) {
        // Section offsets (cumulative row indices):
        // [0, theme_count)       → Theme
        // [markdown_start]       → Markdown: show_line_numbers
        // [panels_start]         → Panels: show_file_tree
        // [panels_start + 1]     → Panels: tree_position left
        // [panels_start + 2]     → Panels: tree_position right
        // [search_start]         → Search: full_line preview
        // [search_start + 1]     → Search: snippet preview
        // [mermaid_start]        → Mermaid: mode Auto
        // [mermaid_start + 1]    → Mermaid: mode Text only
        // [mermaid_start + 2]    → Mermaid: mode Image only
        // [mermaid_start + 3]    → Mermaid: text backend Auto
        // [mermaid_start + 4]    → Mermaid: text backend Sugiyama
        // [mermaid_start + 5]    → Mermaid: text backend Native
        const MARKDOWN_ROWS: usize = 1; // "Show line numbers"
        const PANELS_ROWS: usize = 3; // "Show file tree", "Tree left", "Tree right"
        const SEARCH_ROWS: usize = 2; // "Full line preview", "Snippet preview"
        let theme_count = Theme::ALL.len();
        let markdown_start = theme_count;
        let panels_start = markdown_start + MARKDOWN_ROWS;
        let search_start = panels_start + PANELS_ROWS;
        let mermaid_start = search_start + SEARCH_ROWS;

        if cursor < theme_count {
            let theme = Theme::ALL[cursor];
            self.theme = theme;
            self.palette = Palette::from_theme(theme);
            self.tokens = crate::theme::Tokens::from_theme(theme);
            self.rerender_all_tabs();
            self.persist_config();
        } else if cursor == markdown_start {
            self.show_line_numbers = !self.show_line_numbers;
            self.persist_config();
        } else if cursor == panels_start {
            // Toggle live visibility and persist it as the startup preference so
            // the bullet always reflects the panel's actual state.
            self.tree_hidden = !self.tree_hidden;
            self.show_file_tree = !self.tree_hidden;
            if self.tree_hidden {
                if self.focus == Focus::Tree {
                    self.focus = Focus::Viewer;
                }
            } else {
                self.ensure_tree_discovered();
                self.refresh_git_status();
            }
            self.persist_config();
        } else if cursor == panels_start + 1 {
            self.tree_position = crate::config::TreePosition::Left;
            self.persist_config();
        } else if cursor == panels_start + 2 {
            self.tree_position = crate::config::TreePosition::Right;
            self.persist_config();
        } else if cursor == search_start {
            self.search_preview = crate::config::SearchPreview::FullLine;
            self.persist_config();
        } else if cursor == search_start + 1 {
            self.search_preview = crate::config::SearchPreview::Snippet;
            self.persist_config();
        } else if cursor == mermaid_start {
            // Changing mode clears the cache so existing entries re-render
            // under the new policy on the next draw frame.
            self.mermaid_mode = crate::config::MermaidMode::Auto;
            self.mermaid_cache.clear();
            self.persist_config();
        } else if cursor == mermaid_start + 1 {
            self.mermaid_mode = crate::config::MermaidMode::Text;
            self.mermaid_cache.clear();
            self.persist_config();
        } else if cursor == mermaid_start + 2 {
            self.mermaid_mode = crate::config::MermaidMode::Image;
            self.mermaid_cache.clear();
            self.persist_config();
        } else if cursor == mermaid_start + 3 {
            // Text-mode backend choice — clear the cache so existing
            // diagrams re-render under the new layout backend on the next
            // frame.
            self.mermaid_text_backend = crate::config::MermaidTextBackend::Auto;
            self.mermaid_cache.clear();
            self.persist_config();
        } else if cursor == mermaid_start + 4 {
            self.mermaid_text_backend = crate::config::MermaidTextBackend::Sugiyama;
            self.mermaid_cache.clear();
            self.persist_config();
        } else if cursor == mermaid_start + 5 {
            self.mermaid_text_backend = crate::config::MermaidTextBackend::Native;
            self.mermaid_cache.clear();
            self.persist_config();
        }
    }

    // ── Tree ─────────────────────────────────────────────────────────────────

    /// Handle a key press while the file-tree panel ([`Focus::Tree`]) is focused.
    pub(super) fn handle_tree_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
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

    /// Resolve the second key of a pending `g` chord while the tree is focused.
    ///
    /// Returns `true` when the chord was consumed.
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

    // ── Viewer ────────────────────────────────────────────────────────────────

    /// Resolve the second key of a pending `g` chord in the viewer.
    ///
    /// Returns `true` when the chord was consumed (the caller should return).
    pub(super) fn resolve_g_chord_viewer(&mut self, code: KeyCode) -> bool {
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
    pub(super) fn resolve_y_chord_viewer(&mut self, code: KeyCode) -> bool {
        if code == KeyCode::Char('y') {
            self.yank_current_line();
            return true;
        }
        false
    }

    /// Handle a key press while the markdown viewer ([`Focus::Viewer`]) is focused.
    #[allow(clippy::too_many_lines)]
    pub(super) fn handle_viewer_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
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
                // Cursor on a table → open table modal; on a mermaid block →
                // open mermaid modal. Both modal-openers are no-ops when the
                // cursor is on prose, so the cascade is safe.
                self.try_open_table_modal();
                if self.focus != Focus::TableModal {
                    self.try_open_mermaid_modal();
                }
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
            // `i` opens hybrid live-preview mode by default (since 1.33.0).
            // Users who set `use_hybrid_by_default = false` in config.toml get
            // the pre-1.33.0 behaviour: `i` → fullscreen edtui.
            KeyCode::Char('i') => {
                if self.use_hybrid_by_default {
                    self.enter_hybrid_mode();
                } else {
                    self.enter_edit_mode();
                }
            }
            // `I` is the escape hatch to the legacy fullscreen edtui by default.
            // With `use_hybrid_by_default = false` it becomes the hybrid entry
            // point instead, preserving the pre-1.33.0 mapping.
            KeyCode::Char('I') => {
                if self.use_hybrid_by_default {
                    self.enter_edit_mode();
                } else {
                    self.enter_hybrid_mode();
                }
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
            KeyCode::Char('T') if !self.tabs.is_empty() => {
                let cursor = self.tabs.active_index().unwrap_or(0);
                self.tab_picker = Some(crate::ui::tab_picker::TabPickerState { cursor });
                self.focus = Focus::TabPicker;
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
            // macOS Terminal.app / iTerm2 send Option+Right as the literal
            // bytes `Esc f` (the readline word-forward chord). Crossterm
            // decodes that as `KeyCode::Char('f')` with `KeyModifiers::ALT`.
            // Without this guard the bare-`f` arm below catches it and
            // pops the link picker — which is what the user reported.
            KeyCode::Char('f') if modifiers.contains(KeyModifiers::ALT) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_forward();
                }
            }
            KeyCode::Char('b') if modifiers.contains(KeyModifiers::ALT) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_backward();
                }
            }
            KeyCode::Char('f') => {
                self.open_link_picker();
            }
            KeyCode::Char('o') => {
                self.open_outline_picker();
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
            // Cmd+Left/Right — jump to line start/end. Crossterm only
            // delivers `KeyModifiers::SUPER` when the terminal speaks
            // the Kitty keyboard enhancement protocol (Kitty, recent
            // WezTerm, iTerm2 with the protocol enabled). On terminals
            // that don't, Cmd+arrow is intercepted by the OS or the
            // terminal forwards it as Home/End or Esc+arrow — those
            // paths are handled below.
            KeyCode::Left if modifiers.contains(KeyModifiers::SUPER) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_start();
                }
            }
            KeyCode::Right if modifiers.contains(KeyModifiers::SUPER) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_end();
                }
            }
            // Option/Alt + Left/Right — jump by word. Alt is the macOS
            // convention for Option+arrow on terminals that send the
            // CSI form (`\x1b[1;3D` / `\x1b[1;3C`). Terminals that send
            // Esc+f / Esc+b instead are caught by the `Char('f') if ALT`
            // / `Char('b') if ALT` arms above. Same chord doubles as
            // the cross-platform word-jump shortcut on Linux / Windows
            // terminals that report Alt+arrow.
            //
            // Must precede the bare `Left` / `Right` arms below so the
            // ALT-modified variants don't fall through to single-char
            // movement.
            KeyCode::Left if modifiers.contains(KeyModifiers::ALT) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_backward();
                }
            }
            KeyCode::Right if modifiers.contains(KeyModifiers::ALT) => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_forward();
                }
            }
            // Home / End — jump to line start / end. macOS Terminal
            // forwards Cmd+Left as Home and Cmd+Right as End by default;
            // most other terminals also send Home/End for those native
            // keys. Same handler covers both.
            KeyCode::Home => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_start();
                }
            }
            KeyCode::End => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_end();
                }
            }
            // Vim: `w` next word, `b` previous word, `e` end of word
            // (treated same as `w` for now — landing on the next word's
            // start — since the viewer has no concept of "yank to end of
            // word"). `^` jumps to first non-blank (alias for line start
            // here since we don't track leading-whitespace position).
            KeyCode::Char('w') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_forward();
                }
            }
            KeyCode::Char('b') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_backward();
                }
            }
            KeyCode::Char('e') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_word_forward();
                }
            }
            KeyCode::Char('^') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_start();
                }
            }
            KeyCode::Char('$') => {
                if let Some(tab) = self.tabs.active_tab_mut() {
                    tab.view.cursor_line_end();
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

    // ── Go-to-line bar ────────────────────────────────────────────────────────

    /// Handle a key press while the go-to-line prompt ([`Focus::GotoLine`]) is active.
    pub(super) fn handle_goto_line_key(&mut self, code: KeyCode) {
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
            KeyCode::Char(c) if c.is_ascii_digit() && self.goto_line.input.len() < 9 => {
                self.goto_line.input.push(c);
            }
            _ => {}
        }
    }

    // ── Doc-search bar ────────────────────────────────────────────────────────

    /// Handle a key press while the in-document search bar ([`Focus::DocSearch`]) is active.
    pub(super) fn handle_doc_search_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
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

    // ── Content search modal ──────────────────────────────────────────────────

    /// Handle a key press while the file/content search overlay ([`Focus::Search`]) is open.
    pub(super) fn handle_search_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
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
                self.search.next_result();
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.prev_result();
            }
            KeyCode::Char(c) => {
                self.search.query.push(c);
                self.perform_search();
            }
            _ => {}
        }
    }

    // ── Copy-menu popup ───────────────────────────────────────────────────────

    /// Handle a key press while the copy-path popup ([`Focus::CopyMenu`]) is open.
    pub(super) fn handle_copy_menu_key(&mut self, code: KeyCode) {
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

    // ── Editor ────────────────────────────────────────────────────────────────

    /// Handle a key event while [`Focus::Editor`] is active.
    ///
    /// Two sub-modes:
    /// - **Command-line mode** (`editor.command_line.is_some()`): we capture chars
    ///   ourselves to build an ex command (`:w`, `:q`, etc.).
    /// - **Editing mode**: forward to edtui, but intercept `:` when edtui is in
    ///   Normal mode to start command-line capture.
    pub(super) fn handle_editor_key(&mut self, key: crossterm::event::KeyEvent) {
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
                    let outcome = crate::ui::editor::dispatch_command(editor, &cmd);
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
            if key.code == KeyCode::Char(':') && editor.state.mode == edtui::EditorMode::Normal {
                editor.command_line = Some(String::new());
                editor.status_message = None;
                return;
            }
            // Everything else goes to edtui.
            crate::ui::editor::forward_key_to_edtui(key, &mut editor.state);
        }
    }

    /// Act on the outcome of an ex-command dispatch.
    ///
    /// Must be called *after* `dispatch_command` returns.  `self.tabs` is
    /// fully accessible here because we're back in `&mut self` context.
    pub(super) fn apply_command_outcome(&mut self, outcome: crate::ui::editor::CommandOutcome) {
        match outcome {
            crate::ui::editor::CommandOutcome::Handled => {
                // Nothing to do — `dispatch_command` already set any message.
            }
            crate::ui::editor::CommandOutcome::Save => {
                self.save_editor_content(false);
            }
            crate::ui::editor::CommandOutcome::Close => {
                self.close_editor();
            }
            crate::ui::editor::CommandOutcome::SaveThenClose => {
                self.save_editor_content(true);
            }
        }
    }

    // ── Table modal keys ─────────────────────────────────────────────────────

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
    pub(super) fn handle_table_modal_key(&mut self, code: KeyCode) {
        use crate::ui::table_modal::{max_h_scroll, next_col_boundary, prev_col_boundary};

        if self.pending_chord.take() == Some('g') && code == KeyCode::Char('g') {
            if let Some(s) = self.table_modal.as_mut() {
                s.v_scroll = 0;
                s.h_scroll = 0;
            }
            return;
        }

        let view_height = crate::cast::u16_from_u32(self.tabs.view_height);
        // Derive inner width from the cached modal rect (border is 1 cell on each side).
        // Falls back to 80 before the first draw or in tests that don't call draw.
        let inner_width = self
            .table_modal_rect
            .map_or(80, |r| r.width.saturating_sub(2));

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
                    let total = crate::cast::u16_sat(s.rows.len()) + 3;
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

    // ── Hybrid editor ─────────────────────────────────────────────────────────

    /// Handle a key event while [`Focus::HybridEditor`] is active.
    ///
    /// Two sub-modes mirror the classic editor:
    /// - **Command-line mode** (`hybrid.command_line.is_some()`): we capture
    ///   chars ourselves to build an ex command (`:w`, `:wq`, `:q`, `:q!`).
    /// - **Normal mode**: `:` opens the command line; character input inserts
    ///   into the source; Backspace/Delete remove; Enter inserts a newline.
    ///
    /// Cursor-leave re-parse: the active block index is captured BEFORE the key
    /// is processed.  After the key changes the cursor position,
    /// `recompute_active_block` is called to detect whether the cursor left the
    /// block.  If so, `reparse_and_splice_block` re-parses the just-left block
    /// with pulldown-cmark and splices the result into `view.rendered`.  This is
    /// the "compile" event the user perceives — `**bold**` snaps to bold text,
    /// `# Heading` snaps to the heading glyph, etc.
    pub(super) fn handle_hybrid_key(&mut self, key: crossterm::event::KeyEvent) {
        // Read view_height and theme/palette before any mutable borrows.
        let view_height = self.tabs.view_height as usize;
        let palette = self.palette;
        let theme = self.theme;

        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let Some(hybrid) = tab.hybrid.as_mut() else {
            // Hybrid state was unexpectedly None; snap back to Viewer.
            self.focus = Focus::Viewer;
            return;
        };

        if hybrid.command_line.is_some() {
            // ── Command-line capture mode ────────────────────────────────────
            match key.code {
                KeyCode::Esc => {
                    hybrid.command_line = None;
                    hybrid.status_message = None;
                }
                KeyCode::Backspace => {
                    if let Some(ref mut cmd) = hybrid.command_line {
                        cmd.pop();
                    }
                }
                KeyCode::Enter => {
                    let cmd = hybrid.command_line.take().unwrap_or_default();
                    hybrid.status_message = None;
                    // End the borrow of `tab.hybrid` before calling dispatch_hybrid_command,
                    // which needs `&mut self` for file I/O helpers.
                    let _ = hybrid;
                    self.dispatch_hybrid_command(cmd.trim(), palette, theme);
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut cmd) = hybrid.command_line {
                        cmd.push(c);
                    }
                }
                _ => {}
            }
        } else {
            // ── Normal mode — editing + cursor movement ───────────────────────
            //
            // We snapshot `active_block.index` BEFORE the key is processed so
            // that after processing we can detect whether the cursor left a block.
            // If the index changed, the just-left block is re-parsed via
            // pulldown-cmark and spliced back into `view.rendered`.
            //
            // Borrow structure: `Tab` stores `hybrid: Option<HybridState>` and
            // `view: MarkdownViewState` as disjoint fields.  Rust's NLL allows us
            // to hold `hybrid: &mut HybridState` and `view: &mut MarkdownViewState`
            // simultaneously because they come from different fields of the same
            // struct.  The `:` arm only needs `hybrid`; all other arms need both.

            // Snapshot the active block index before the key lands.
            let prev_block_index = hybrid.active_block.map(|ab| ab.index);

            match key.code {
                KeyCode::Char(':') => {
                    hybrid.command_line = Some(String::new());
                    // No cursor move → no block-leave possible.
                    return;
                }
                KeyCode::Esc => {
                    // Already in normal mode — no-op.
                    return;
                }
                _ => {
                    // All editing and movement keys re-acquire a fresh `&mut tab`
                    // so we can split-borrow `hybrid` and `view` as mutable.
                    let _ = hybrid; // release the outer borrow of `tab.hybrid`
                    if let Some(tab2) = self.tabs.active_tab_mut()
                        && let Some(h) = tab2.hybrid.as_mut()
                    {
                        // Split-borrow: `h` mutably borrows `tab2.hybrid`;
                        // `view` mutably borrows `tab2.view`.  Disjoint — safe.
                        let view = &mut tab2.view;
                        dispatch_hybrid_key(h, view, key, view_height);
                    }
                }
            }

            // ── Cursor-leave re-parse ────────────────────────────────────────
            // After the key was processed, check whether the cursor left the
            // previously active block.  If it did, re-parse that block with
            // pulldown-cmark so markdown styling (bold, headings, code) appears
            // in the just-left block.
            //
            // This is the design's "compile on leave" event: while the cursor is
            // inside a block it shows raw source; on departure it gets the full
            // rendered treatment from pulldown-cmark's structural output.
            if let Some(prev_idx) = prev_block_index
                && let Some(tab2) = self.tabs.active_tab_mut()
            {
                let new_idx = tab2
                    .hybrid
                    .as_ref()
                    .and_then(|h| h.active_block)
                    .map(|ab| ab.index);
                if new_idx != Some(prev_idx) {
                    // Cursor moved to a different block — re-parse the one we left.
                    if let Some(h) = tab2.hybrid.as_ref() {
                        crate::ui::hybrid_editor::reparse_and_splice_block(
                            h,
                            &mut tab2.view,
                            prev_idx,
                            &palette,
                            theme,
                        );
                    }
                    // After splicing the block list may have grown/shrunk;
                    // recompute the active block from the current cursor byte.
                    if let Some(h) = tab2.hybrid.as_mut() {
                        crate::ui::hybrid_editor::recompute_active_block(h, &tab2.view);
                    }
                }
            }
        }
    }

    /// Dispatch a hybrid-mode ex command (the text after `:`).
    ///
    /// Called from `handle_hybrid_key` after the command string is complete.
    /// Needs `&mut self` so it can initiate async saves and call `exit_hybrid_mode`.
    fn dispatch_hybrid_command(
        &mut self,
        cmd: &str,
        palette: crate::theme::Palette,
        theme: crate::theme::Theme,
    ) {
        match cmd {
            "w" => {
                // Full re-parse before save to eliminate any incremental drift,
                // then kick off the async write.
                if let Some(tab) = self.tabs.active_tab_mut()
                    && let Some(h) = tab.hybrid.as_mut()
                {
                    crate::ui::hybrid_editor::full_reparse(h, &mut tab.view, &palette, theme);
                    h.status_message = Some("saving…".to_string());
                }
                self.save_hybrid_content(false);
            }
            "wq" => {
                if let Some(tab) = self.tabs.active_tab_mut()
                    && let Some(h) = tab.hybrid.as_mut()
                {
                    crate::ui::hybrid_editor::full_reparse(h, &mut tab.view, &palette, theme);
                    h.close_after_save = true;
                }
                self.save_hybrid_content(true);
            }
            "q" => {
                // Refuse to quit if there are unsaved edits — mirror vim semantics.
                let dirty = self
                    .tabs
                    .active_tab()
                    .and_then(|t| t.hybrid.as_ref())
                    .map(|h| h.is_dirty())
                    .unwrap_or(false);
                if dirty {
                    if let Some(tab) = self.tabs.active_tab_mut()
                        && let Some(h) = tab.hybrid.as_mut()
                    {
                        h.status_message = Some("unsaved changes — use :q! to discard".to_string());
                    }
                } else {
                    self.exit_hybrid_mode();
                }
            }
            "q!" => {
                self.exit_hybrid_mode();
            }
            other => {
                if let Some(tab) = self.tabs.active_tab_mut()
                    && let Some(h) = tab.hybrid.as_mut()
                {
                    h.status_message = Some(format!("unknown command: :{other}"));
                }
            }
        }
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
    pub(super) fn handle_table_modal_mouse(&mut self, m: crossterm::event::MouseEvent) {
        use crate::ui::table_modal::{max_h_scroll, next_col_boundary, prev_col_boundary};
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        let col = m.column;
        let row = m.row;
        // If the rect hasn't been populated yet (first frame), treat the
        // event as inside so we don't inadvertently close on the first click.
        let inside = self.table_modal_rect.is_none_or(|r| contains(r, col, row));

        // view_height is used by max_h_scroll to determine the visible horizontal
        // extent; we reuse the viewer's stored height as an approximation.
        let view_height = crate::cast::u16_from_u32(self.tabs.view_height);

        match m.kind {
            // Click inside the modal is a no-op; click outside closes it.
            MouseEventKind::Down(MouseButton::Left) if !inside => {
                self.close_table_modal();
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
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Dispatch a key event in hybrid normal mode (editing + cursor movement).
///
/// Called from `handle_hybrid_key` after splitting the `Tab` into a mutable
/// `HybridState` borrow and a mutable `MarkdownViewState` borrow.  Routing the
/// dispatch through a free function keeps the per-arm logic out of the key
/// handler and avoids repeating the `if let Some(tab) … if let Some(h) …`
/// boilerplate for every key.
///
/// # Arguments
///
/// * `hybrid`      – mutable hybrid state (cursor, source buffer, active_block).
/// * `view`        – mutable view state (block list modified by editing helpers).
/// * `code`        – the key code that triggered this call.
/// * `view_height` – viewport height in lines (used for Page Up/Down).
fn dispatch_hybrid_key(
    hybrid: &mut crate::ui::hybrid_editor::HybridState,
    view: &mut crate::ui::markdown_view::MarkdownViewState,
    key: crossterm::event::KeyEvent,
    view_height: usize,
) {
    use crate::ui::hybrid_editor::{
        delete_char_after, delete_char_before, delete_to_line_end, delete_to_line_start,
        delete_word_after, delete_word_before, insert_char, move_cursor_doc_end,
        move_cursor_doc_start, move_cursor_down, move_cursor_left, move_cursor_line_end,
        move_cursor_line_start, move_cursor_page_down, move_cursor_page_up, move_cursor_right,
        move_cursor_up, move_cursor_word_left, move_cursor_word_right,
    };
    use crossterm::event::KeyModifiers;

    let code = key.code;
    let m = key.modifiers;
    // Aggregate modifier flags so the match below can read like a keymap.
    //
    // Terminal emulators are wildly inconsistent about which modifier flag
    // they tag macOS Option with — crossterm parses it as `ALT` for some
    // terminals and `META` for others, depending on the underlying CSI
    // sequence the terminal sends. macOS Cmd similarly arrives as `SUPER`
    // when it arrives at all (most terminals swallow it). Normalising both
    // here keeps the match below stable across iTerm2, Ghostty, Wezterm,
    // Apple Terminal, and Alacritty.
    let alt = m.contains(KeyModifiers::ALT) || m.contains(KeyModifiers::META);
    let cmd = m.contains(KeyModifiers::SUPER);
    let ctrl = m.contains(KeyModifiers::CONTROL);

    match code {
        // ── Word-level motion (Option/Alt OR Ctrl) ────────────────────────────
        // Ctrl+arrow is the cross-platform convention (VS Code, browsers,
        // GitHub). Always works regardless of terminal Option-key quirks.
        KeyCode::Left if alt || ctrl => move_cursor_word_left(hybrid, view),
        KeyCode::Right if alt || ctrl => move_cursor_word_right(hybrid, view),

        // ── Line-level motion (Cmd) ───────────────────────────────────────────
        // Most macOS terminals capture Cmd themselves so these usually never
        // fire; the Home/End and Ctrl+A/E paths below are the reliable ones.
        KeyCode::Left if cmd => move_cursor_line_start(hybrid, view),
        KeyCode::Right if cmd => move_cursor_line_end(hybrid, view),

        // ── Document-level motion (Cmd) ───────────────────────────────────────
        KeyCode::Up if cmd => move_cursor_doc_start(hybrid, view),
        KeyCode::Down if cmd => move_cursor_doc_end(hybrid, view),

        // ── Word-level deletion ───────────────────────────────────────────────
        KeyCode::Backspace if alt => delete_word_before(hybrid, view),
        KeyCode::Delete if alt => delete_word_after(hybrid, view),

        // ── Line-level deletion ───────────────────────────────────────────────
        KeyCode::Backspace if cmd => delete_to_line_start(hybrid, view),

        // ── Unix-style Ctrl shortcuts (work everywhere, including over SSH) ──
        KeyCode::Char('a') if ctrl => move_cursor_line_start(hybrid, view),
        KeyCode::Char('e') if ctrl => move_cursor_line_end(hybrid, view),
        KeyCode::Char('w') if ctrl => delete_word_before(hybrid, view),
        KeyCode::Char('u') if ctrl => delete_to_line_start(hybrid, view),
        KeyCode::Char('k') if ctrl => delete_to_line_end(hybrid, view),

        // ── Readline-style Alt+letter shortcuts ───────────────────────────────
        // macOS Terminal / iTerm2 with "Use Option as Meta" send Option+Right
        // as `Alt+f` (Char('f') + ALT) and Option+Left as `Alt+b`, mirroring
        // GNU readline's `forward-word` / `backward-word` bindings — never as
        // `Alt+Right` / `Alt+Left`. Without these arms the Char(_) if alt
        // catch-all below would silently swallow them.
        KeyCode::Char('f') if alt => move_cursor_word_right(hybrid, view),
        KeyCode::Char('b') if alt => move_cursor_word_left(hybrid, view),
        KeyCode::Char('d') if alt => delete_word_after(hybrid, view),

        // ── Editing keys ──────────────────────────────────────────────────────
        // Characters are inserted literally at the cursor.  The active block
        // renders raw while the cursor is inside it; pulldown-cmark styling
        // only appears after the cursor leaves (cursor-leave re-parse).
        // Skip insertion when a non-Shift modifier is held — those combinations
        // either matched a shortcut above or are intentionally unbound.
        KeyCode::Char(_) if alt || cmd || ctrl => {}
        KeyCode::Char(c) => insert_char(hybrid, view, c),
        KeyCode::Enter => insert_char(hybrid, view, '\n'),
        KeyCode::Tab => insert_char(hybrid, view, '\t'),
        KeyCode::Backspace => delete_char_before(hybrid, view),
        KeyCode::Delete => delete_char_after(hybrid, view),

        // ── Plain cursor movement ─────────────────────────────────────────────
        KeyCode::Left => move_cursor_left(hybrid, view),
        KeyCode::Right => move_cursor_right(hybrid, view),
        KeyCode::Down => move_cursor_down(hybrid, view),
        KeyCode::Up => move_cursor_up(hybrid, view),
        KeyCode::PageDown => move_cursor_page_down(hybrid, view, view_height),
        KeyCode::PageUp => move_cursor_page_up(hybrid, view, view_height),
        KeyCode::Home => move_cursor_line_start(hybrid, view),
        KeyCode::End => move_cursor_line_end(hybrid, view),

        // Unknown keys are no-ops.
        _ => {}
    }
}
