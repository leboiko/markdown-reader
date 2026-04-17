/// Search implementations: file/content search and in-document search.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::action::Action;
use crate::fs::discovery::FileEntry;
use crate::ui::search_modal::{RESULT_CAP, SearchMode, SearchResult, build_preview, smartcase_is_sensitive};
use std::sync::{Arc, atomic::Ordering};

impl App {
    // ── Content / filename search ─────────────────────────────────────────────

    /// Start or refresh the current search.
    ///
    /// Filename searches run synchronously on the main thread.  Content
    /// searches are dispatched to a background thread to avoid blocking the
    /// event loop.  A monotonically increasing generation counter ensures that
    /// results from a superseded query are silently discarded.
    #[allow(clippy::too_many_lines)]
    pub(super) fn perform_search(&mut self) {
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

    /// Open the file from the currently highlighted search result, jumping to
    /// the first match line in content-search mode.
    pub(super) fn confirm_search(&mut self) {
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
        let jump_to_source = result.first_match_line.map(crate::cast::u32_sat);

        self.open_or_focus(result.path, true, jump_to_source);
    }

    // ── In-document search ────────────────────────────────────────────────────

    /// Rebuild match-line list for the current in-document search query and
    /// jump the cursor to the first match.
    pub(super) fn perform_doc_search(&mut self) {
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

        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
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

    /// Advance to the next in-document search match, wrapping around.
    ///
    /// Sets `cursor_line` to the match line and calls `scroll_to_cursor` so
    /// subsequent `j`/`k` presses move from the correct row.
    pub(super) fn doc_search_next(&mut self) {
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

    /// Retreat to the previous in-document search match, wrapping around.
    ///
    /// Sets `cursor_line` to the match line and calls `scroll_to_cursor` so
    /// subsequent `j`/`k` presses move from the correct row.
    pub(super) fn doc_search_prev(&mut self) {
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
}
