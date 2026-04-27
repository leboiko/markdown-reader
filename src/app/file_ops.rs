/// File-operation helpers: open, reload, save, edit-mode transitions.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::action::Action;
use crate::markdown::DocBlock;
use crate::ui::tabs::OpenOutcome;
use std::time::Instant;

// ── Private helpers for hybrid-mode byte ↔ editor-position translation ────────

/// Convert a source byte offset to an edtui `(row, col)` position using the
/// pre-computed `line_boundaries` table.
///
/// `line_boundaries[i]` is the byte offset of the start of line `i`.  We binary-
/// search for the line that contains `byte` and compute the column offset within
/// that line.  Both values are returned as `usize` for use with
/// [`edtui::Index2::new`].
fn byte_to_editor_pos(line_boundaries: &[usize], byte: usize) -> (usize, usize) {
    // Binary search: find the largest boundary index whose value is <= byte.
    let row = match line_boundaries.binary_search(&byte) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let col = byte.saturating_sub(line_boundaries.get(row).copied().unwrap_or(0));
    (row, col)
}

/// Convert an edtui `(row, col)` position back to a source byte offset using the
/// pre-computed `line_boundaries` table.
///
/// Clamps to the end of the source when `row` or `col` is out of range.
fn editor_pos_to_byte(line_boundaries: &[usize], row: usize, col: usize) -> usize {
    let line_start = line_boundaries.get(row).copied().unwrap_or_else(|| {
        // Row past end of document — clamp to last boundary.
        line_boundaries.last().copied().unwrap_or(0)
    });
    line_start + col
}

/// Extract `(source_byte_start, source_byte_end)` from any [`DocBlock`] variant.
///
/// Returns `(0, 0)` if the block has zero-length ranges (shouldn't happen after
/// the post-render fixup pass, but is safe to call unconditionally).
fn block_byte_range_of(block: &DocBlock) -> (usize, usize) {
    match block {
        DocBlock::Text {
            source_byte_start,
            source_byte_end,
            ..
        } => (*source_byte_start as usize, *source_byte_end as usize),
        DocBlock::Mermaid {
            source_byte_start,
            source_byte_end,
            ..
        } => (*source_byte_start as usize, *source_byte_end as usize),
        DocBlock::Table(t) => (t.source_byte_start as usize, t.source_byte_end as usize),
    }
}

impl App {
    // ── Tree width helpers ────────────────────────────────────────────────────

    /// Shrink the tree panel by 5 percentage points (minimum 10 %).
    pub(super) fn shrink_tree(&mut self) {
        // No-op when the tree is hidden: there is nothing visible to resize.
        if !self.tree_hidden {
            self.tree_width_pct = self.tree_width_pct.saturating_sub(5).max(10);
        }
    }

    /// Grow the tree panel by 5 percentage points (maximum 80 %).
    pub(super) fn grow_tree(&mut self) {
        // No-op when the tree is hidden: there is nothing visible to resize.
        if !self.tree_hidden {
            self.tree_width_pct = (self.tree_width_pct + 5).min(80);
        }
    }

    // ── Tab helpers ───────────────────────────────────────────────────────────

    /// Commit any in-progress doc-search and switch focus back to Viewer before
    /// performing a tab switch.
    pub(super) fn commit_doc_search_if_active(&mut self) {
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

    /// Close the mermaid modal if open, restoring focus to Viewer.
    pub fn close_mermaid_modal(&mut self) {
        if self.mermaid_modal.is_some() {
            self.mermaid_modal = None;
            self.mermaid_modal_rect = None;
            self.focus = Focus::Viewer;
        }
    }

    /// Switch to the next tab, committing any active doc-search first.
    pub(super) fn switch_to_next_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
        self.close_mermaid_modal();
        self.tabs.next();
    }

    /// Switch to the previous tab, committing any active doc-search first.
    pub(super) fn switch_to_prev_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
        self.close_mermaid_modal();
        self.tabs.prev();
    }

    // ── File opening ──────────────────────────────────────────────────────────

    /// Open the selected tree item in the active tab (replacing its content).
    pub(super) fn open_in_active_tab(&mut self) {
        self.open_selected_file(false);
    }

    /// Open `path` in a tab, optionally jumping to a source line after load.
    ///
    /// Delegates to [`open_or_focus_named`] with no display-name override.
    /// When a custom tab label is needed (e.g. `"<stdin>"`), use
    /// [`open_or_focus_named`] directly.
    ///
    /// # Arguments
    ///
    /// * `path`           – file to open (directories are silently ignored).
    /// * `new_tab`        – when `true`, push or focus a tab; when `false`,
    ///   replace the active tab's content.
    /// * `jump_to_source` – when `Some(line)`, position the cursor at the
    ///   rendered logical line matching the given 0-indexed source line after
    ///   the file loads (or immediately when already open).
    pub fn open_or_focus(
        &mut self,
        path: PathBuf,
        new_tab: bool,
        jump_to_source: Option<u32>,
    ) {
        self.open_or_focus_named(path, new_tab, jump_to_source, None);
    }

    /// Like [`open_or_focus`] but allows an explicit `display_name` for the tab bar.
    pub fn open_or_focus_named(
        &mut self,
        path: PathBuf,
        new_tab: bool,
        jump_to_source: Option<u32>,
        display_name: Option<String>,
    ) {
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
                    && let Some(logical) = crate::markdown::logical_line_at_source(
                        &tab.view.rendered,
                        source_line,
                        &tab.view.text_layouts,
                    )
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
                        display_name,
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
    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn apply_file_loaded(
        &mut self,
        path: PathBuf,
        content: String,
        _new_tab: bool,
        display_name: Option<String>,
    ) {
        // Use the caller-supplied display name when present (e.g. `<stdin>`);
        // otherwise fall back to the file's basename.  This lets stdin tabs
        // show a conventional Unix sentinel in the tab bar instead of the
        // generated temp-file name.
        let name = display_name.unwrap_or_else(|| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

        // Find the placeholder tab that open_or_focus reserved (it has
        // current_path set but no content yet) and load the file into it.
        let palette = self.palette;
        let loaded = self
            .tabs
            .find_tab_by_path_mut(&path)
            .filter(|t| t.view.content.is_empty())
            .is_some();

        if loaded {
            // SAFETY: the `is_some()` guard above guarantees a tab exists.
            let tab = self
                .tabs
                .find_tab_by_path_mut(&path)
                .expect("tab must exist after is_some() guard");
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
                    && let Some(logical) = crate::markdown::logical_line_at_source(
                        &tab.view.rendered,
                        source_line,
                        &tab.view.text_layouts,
                    )
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

    /// Open the currently selected tree file in a new or active tab.
    ///
    /// When `new_tab` is `false`, the active tab's content is replaced.
    pub(super) fn open_selected_file(&mut self, new_tab: bool) {
        let Some(path) = self.tree.selected_path().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.open_or_focus(path, new_tab, None);
    }

    /// Reload every open tab whose path is in the `changed` set.
    ///
    /// Preserves each tab's scroll offset (clamped to the new line count).
    /// Spawn a background read for each tab whose path is in `changed`.
    ///
    /// Each read completes asynchronously and arrives as [`Action::FileReloaded`].
    pub(super) fn reload_changed_tabs(&mut self, changed: &[PathBuf]) {
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
    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn apply_file_reloaded(&mut self, path: PathBuf, content: String) {
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

        // Close any open block-level modal if it was on the reloaded tab —
        // its cached state (table rows, mermaid block_id) is stale once the
        // file content changed.
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
        if let Some(modal) = &self.mermaid_modal {
            let tab_id = modal.tab_id;
            let is_reloaded = self
                .tabs
                .tabs
                .iter()
                .any(|t| t.id == tab_id && t.view.current_path.as_deref() == Some(&*path));
            if is_reloaded {
                self.close_mermaid_modal();
            }
        }
    }

    // ── Editor lifecycle ──────────────────────────────────────────────────────

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
        let target_source_line = crate::markdown::source_line_at(
            &tab.view.rendered,
            tab.view.cursor_line,
            &tab.view.text_layouts,
            &tab.view.table_layouts,
        );
        let source_lines_total = content.split('\n').count();
        let target_row = (target_source_line as usize).min(source_lines_total.saturating_sub(1));
        let mut editor = crate::ui::editor::TabEditor::new(content);
        editor.state.cursor = edtui::Index2::new(target_row, 0);
        tab.editor = Some(editor);
        self.focus = Focus::Editor;
    }

    /// Initiate an async write of the active tab's editor buffer to disk.
    ///
    /// Uses an atomic rename via `tempfile` to avoid partial writes.  On
    /// completion, sends [`Action::FileSaved`] or [`Action::FileSaveError`].
    ///
    /// If `close_after_save` is `true`, the editor will be closed in the
    /// `FileSaved` handler (`:wq` behaviour).
    pub(super) fn save_editor_content(&mut self, close_after_save: bool) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let Some(editor) = tab.editor.as_ref() else {
            return;
        };
        let Some(path) = tab.view.current_path.clone() else {
            return;
        };

        let content = crate::ui::editor::extract_text(&editor.state);
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
    pub fn close_editor(&mut self) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.editor = None;
        }
        self.focus = Focus::Viewer;
    }

    /// Apply a successful editor save.
    ///
    /// Updates the editor baseline so dirty detection is correct, refreshes
    /// `tab.view.content` with the saved text, and closes the editor if
    /// `close_after_save` was set (`:wq` path).
    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn apply_file_saved(&mut self, path: PathBuf, saved_content: String) {
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
                saved_content.clone_into(&mut editor.baseline);
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

        // Also handle the hybrid-mode save path: update hybrid.baseline and
        // optionally exit hybrid mode (`:wq`).  `apply_hybrid_saved` is a
        // no-op when the tab is not in hybrid mode.
        self.apply_hybrid_saved(&path, &saved_content);

        self.last_file_save_at = Some((path, Instant::now()));

        // Refresh git status so the file tree recolors to reflect the save
        // (new → modified, or modified → clean if the edit was reverted).
        // The watcher suppression above prevents a FilesChanged action from
        // firing, which is where the refresh normally hooks in.
        self.refresh_git_status();
    }

    // ── Hybrid editor lifecycle ───────────────────────────────────────────────

    /// Enter hybrid live-preview editing mode for the currently active tab.
    ///
    /// Requires the tab to have a `current_path` set (i.e., it was loaded from
    /// disk).  Constructs a [`crate::ui::hybrid_editor::HybridState`] from the
    /// current source buffer, positions the cursor at the source byte that
    /// corresponds to the viewer's current `cursor_line`, and switches focus to
    /// [`Focus::HybridEditor`].
    ///
    /// The mode is read-only in sub-phase 4.  Arrow keys and editing keystrokes
    /// are no-ops; only `:q` does anything.  Sub-phase 5 adds cursor movement
    /// and sub-phase 6 adds editing.
    pub fn enter_hybrid_mode(&mut self) {
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        // Only enter hybrid mode when we have a real path on disk.
        if tab.view.current_path.is_none() {
            return;
        }

        let source = tab.view.content.clone();
        let mut state = crate::ui::hybrid_editor::HybridState::from_source(&source);

        // Compute the initial cursor byte offset from the viewer's current
        // visual position.  `visual_to_byte` maps (cursor_line, cursor_col) →
        // a source byte offset for Text blocks.  Fall back to byte 0 when the
        // cursor is on a Mermaid or Table block (sub-phase 4 doesn't handle those).
        let cursor_byte = crate::markdown::cursor_bridge::visual_to_byte(
            &tab.view.rendered,
            &tab.view.text_layouts,
            tab.view.cursor_line,
            // cursor_col is already u16 (display-column units, unicode-width).
            tab.view.cursor_col,
        )
        .unwrap_or(0);

        // Map the byte offset to an edtui cursor row/col so the cursor
        // tracks the same logical position in the source.
        let (cursor_row, cursor_col_in_line) =
            byte_to_editor_pos(&state.line_boundaries, cursor_byte);
        state.editor_state.cursor = edtui::Index2::new(cursor_row, cursor_col_in_line);

        // Record which block the cursor is in so sub-phase 5 can reveal it.
        if !tab.view.rendered.is_empty() {
            let block_idx = crate::markdown::cursor_bridge::byte_offset_to_block(
                &tab.view.rendered,
                cursor_byte,
            );
            let (start, end) = block_byte_range_of(&tab.view.rendered[block_idx]);
            state.active_block = Some(crate::ui::hybrid_editor::BlockSourceRange {
                index: block_idx,
                start_byte: start,
                end_byte: end,
            });
        }

        tab.hybrid = Some(state);
        self.focus = Focus::HybridEditor;
    }

    /// Exit hybrid live-preview editing mode, restoring viewer focus.
    ///
    /// Drops `tab.hybrid` and switches focus back to [`Focus::Viewer`].  The
    /// viewer cursor is restored to the visual position that corresponds to the
    /// hybrid cursor's last byte offset (best-effort round-trip via
    /// `byte_to_visual`).
    pub fn exit_hybrid_mode(&mut self) {
        let Some(tab) = self.tabs.active_tab_mut() else {
            self.focus = Focus::Viewer;
            return;
        };

        // Extract the final cursor byte so we can restore the viewer position.
        let final_byte = tab.hybrid.as_ref().map(|h| {
            editor_pos_to_byte(
                &h.line_boundaries,
                h.editor_state.cursor.row,
                h.editor_state.cursor.col,
            )
        });

        // Drop hybrid state first.
        tab.hybrid = None;
        self.focus = Focus::Viewer;

        // Attempt to restore the viewer cursor from the hybrid cursor's byte.
        if let Some(byte) = final_byte
            && let Some((visual_row, _visual_col)) = crate::markdown::cursor_bridge::byte_to_visual(
                &tab.view.rendered,
                &tab.view.text_layouts,
                byte,
            )
        {
            let vh = self.tabs.view_height;
            if let Some(tab2) = self.tabs.active_tab_mut() {
                tab2.view.cursor_line = visual_row;
                tab2.view.scroll_to_cursor(vh);
            }
        }
    }

    // ── Hybrid editor save ────────────────────────────────────────────────────

    /// Initiate an async write of the active tab's hybrid source buffer to disk.
    ///
    /// Mirrors [`save_editor_content`] but reads from `hybrid.source` instead of
    /// the edtui buffer.  On completion, sends [`Action::FileSaved`] or
    /// [`Action::FileSaveError`].
    ///
    /// If `close_after_save` is `true`, `hybrid.close_after_save` is set so the
    /// `FileSaved` handler knows to call `exit_hybrid_mode` (`:wq` behaviour).
    ///
    /// # Arguments
    ///
    /// * `close_after_save` – when `true`, exit hybrid mode after the write
    ///   succeeds (`:wq` path).
    pub(super) fn save_hybrid_content(&mut self, close_after_save: bool) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let Some(hybrid) = tab.hybrid.as_ref() else {
            return;
        };
        let Some(path) = tab.view.current_path.clone() else {
            return;
        };

        let content = hybrid.source.clone();
        let Some(tx) = self.action_tx.clone() else {
            return;
        };

        let path_for_closure = path.clone();
        tokio::task::spawn_blocking(move || {
            let path = path_for_closure;
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

        // Record save time before the async task finishes to start the watcher
        // grace window early (avoids a race where the watcher fires before the
        // action arrives).
        self.last_file_save_at = Some((path, Instant::now()));

        if close_after_save
            && let Some(tab2) = self.tabs.active_tab_mut()
            && let Some(h) = tab2.hybrid.as_mut()
        {
            h.close_after_save = true;
        }
    }

    /// Apply a completed async hybrid-mode save.
    ///
    /// Updates `hybrid.baseline` so dirty detection is correct, refreshes
    /// `tab.view.content` with the saved text, and exits hybrid mode if
    /// `close_after_save` was set (`:wq` path).
    ///
    /// # Note
    ///
    /// This is called by `apply_file_saved`, which already handles the `editor`
    /// (fullscreen edtui) side.  The hybrid side is a parallel update.
    pub(super) fn apply_hybrid_saved(&mut self, path: &std::path::Path, saved_content: &str) {
        let palette = self.palette;
        let theme = self.theme;
        let mut should_exit = false;

        for tab in self.tabs.iter_mut() {
            if tab.view.current_path.as_deref() != Some(path) {
                continue;
            }
            if let Some(h) = tab.hybrid.as_mut() {
                // Sync baseline so is_dirty() returns false after save.
                saved_content.clone_into(&mut h.baseline);
                h.status_message = Some("saved".to_string());
                should_exit = h.close_after_save;
            }
            // Refresh the view content so the viewer shows the saved file.
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let scroll = tab.view.scroll_offset;
            tab.view.load(
                path.to_path_buf(),
                name,
                saved_content.to_string(),
                &palette,
                theme,
            );
            tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));
            break;
        }

        if should_exit {
            self.exit_hybrid_mode();
        }
    }

    // ── Link picker ───────────────────────────────────────────────────────────

    /// Build the link picker from the active tab's internal `#anchor` links,
    /// deduplicated by anchor, and open it.
    ///
    /// Sort order is **by TARGET heading line**, not by where the link
    /// text appears in the source. Rationale: the picker's purpose is
    /// "jump somewhere in the document" — the natural mental model is
    /// "things I can jump to, in document order." When an intro paragraph
    /// links to a section at the END of the doc (e.g., "see also: [last
    /// section]"), that link should sit near the BOTTOM of the picker
    /// (where its target lives), not near the top (where its text was
    /// written). Otherwise users press `j/k` expecting to walk the doc
    /// top-to-bottom and end up jumping randomly across sections.
    ///
    /// Within a target line, ties are broken by source position so
    /// multiple links to the same heading retain a deterministic order.
    pub(super) fn open_link_picker(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        // Pre-build an `anchor → target_line` lookup so we can both filter
        // (`has_target`) and sort (by line) without rescanning
        // `heading_anchors` per link.
        let mut anchor_line: std::collections::HashMap<&str, u32> =
            std::collections::HashMap::with_capacity(tab.view.heading_anchors.len());
        for a in &tab.view.heading_anchors {
            // First-occurrence wins on duplicate slugs (matches the
            // jump behaviour in `link_picker::handle_key`).
            anchor_line.entry(a.anchor.as_str()).or_insert(a.line);
        }

        // Walk source-ordered links, dedup by anchor, keep only those
        // resolving to a known heading. Filter BEFORE dedup so a stray
        // missing-target link can't shadow a later same-anchor link
        // that DOES resolve.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut items: Vec<(u32, u16, crate::ui::link_picker::LinkPickerItem)> = Vec::new();

        for link in &tab.view.links {
            if !link.url.starts_with('#') {
                continue;
            }
            let anchor = &link.url[1..];
            let Some(&target_line) = anchor_line.get(anchor) else {
                continue;
            };
            if !seen.insert(anchor.to_string()) {
                continue;
            }
            items.push((
                target_line,
                link.col_start,
                crate::ui::link_picker::LinkPickerItem {
                    text: link.text.clone(),
                    anchor: anchor.to_string(),
                },
            ));
        }

        if items.is_empty() {
            return;
        }

        // Primary key: target heading line (ascending = doc top to bottom).
        // Tie-break: link's source col_start, so deterministic.
        items.sort_by_key(|(line, col, _)| (*line, *col));
        let items: Vec<_> = items.into_iter().map(|(_, _, it)| it).collect();

        self.link_picker = Some(crate::ui::link_picker::LinkPickerState { cursor: 0, items });
        self.focus = Focus::LinkPicker;
    }

    /// Open the outline (heading) picker for the active tab.
    ///
    /// Collects every heading anchor from the active tab's rendered blocks and
    /// opens the `OutlinePicker` overlay. No-ops when there is no active tab.
    /// When the document contains no headings, the picker still opens so the
    /// user sees the "no headings" placeholder message.
    pub(super) fn open_outline_picker(&mut self) {
        let Some(picker) = crate::ui::outline_picker::OutlinePickerState::build(self) else {
            return;
        };
        self.outline_picker = Some(picker);
        self.focus = Focus::OutlinePicker;
    }

    /// Expand every ancestor directory of `file` in the tree and select the file.
    ///
    /// Delegates to [`FileTreeState::reveal_path`], which handles the ancestor
    /// walk, flat-list rebuild, and selection update in one step.
    pub(super) fn expand_and_select(&mut self, file: &Path) {
        self.tree.reveal_path(file);
    }
}
