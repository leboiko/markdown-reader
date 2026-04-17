/// File-operation helpers: open, reload, save, edit-mode transitions.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::action::Action;
use crate::ui::tabs::OpenOutcome;
use std::time::Instant;

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

    /// Switch to the next tab, committing any active doc-search first.
    pub(super) fn switch_to_next_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
        self.tabs.next();
    }

    /// Switch to the previous tab, committing any active doc-search first.
    pub(super) fn switch_to_prev_tab(&mut self) {
        self.commit_doc_search_if_active();
        self.close_table_modal();
        self.tabs.prev();
    }

    // ── File opening ──────────────────────────────────────────────────────────

    /// Open the selected tree item in the active tab (replacing its content).
    pub(super) fn open_in_active_tab(&mut self) {
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
    /// * `path`           – file to open (directories are silently ignored).
    /// * `new_tab`        – when `true`, push or focus a tab; when `false`,
    ///   replace the active tab's content.
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
    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn apply_file_loaded(&mut self, path: PathBuf, content: String, _new_tab: bool) {
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
        let target_source_line =
            crate::markdown::source_line_at(&tab.view.rendered, tab.view.cursor_line);
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

        self.last_file_save_at = Some((path, Instant::now()));

        // Refresh git status so the file tree recolors to reflect the save
        // (new → modified, or modified → clean if the edit was reverted).
        // The watcher suppression above prevents a FilesChanged action from
        // firing, which is where the refresh normally hooks in.
        self.refresh_git_status();
    }

    // ── Link picker ───────────────────────────────────────────────────────────

    /// Build the link picker from the active tab's internal `#anchor` links,
    /// deduplicated by anchor, and open it.
    pub(super) fn open_link_picker(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        // Collect unique anchors preserving first-occurrence order.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut items: Vec<crate::ui::link_picker::LinkPickerItem> = Vec::new();

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
                items.push(crate::ui::link_picker::LinkPickerItem {
                    text: link.text.clone(),
                    anchor: anchor.to_string(),
                });
            }
        }

        if items.is_empty() {
            return;
        }

        self.link_picker = Some(crate::ui::link_picker::LinkPickerState { cursor: 0, items });
        self.focus = Focus::LinkPicker;
    }

    /// Expand every ancestor directory of `file` in the tree and select the file.
    ///
    /// Delegates to [`FileTreeState::reveal_path`], which handles the ancestor
    /// walk, flat-list rebuild, and selection update in one step.
    pub(super) fn expand_and_select(&mut self, file: &Path) {
        self.tree.reveal_path(file);
    }
}
