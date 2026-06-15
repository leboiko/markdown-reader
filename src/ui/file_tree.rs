use crate::app::App;
use crate::fs::discovery::FileEntry;
use crate::fs::git_status::GitFileStatus;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Persistent UI state for the file-tree panel.
#[derive(Debug, Default)]
pub struct FileTreeState {
    /// Hierarchical tree of discovered markdown files and directories.
    pub entries: Vec<FileEntry>,
    /// Flattened, visible list derived from `entries` and `expanded`.
    pub flat_items: Vec<FlatItem>,
    /// Ratatui list selection state (tracks scroll and highlighted index).
    pub list_state: ListState,
    /// Set of directory paths that are currently expanded.
    pub expanded: HashSet<PathBuf>,
    /// Git working-tree status, keyed by absolute path.
    ///
    /// Populated on startup and refreshed on `FilesChanged`. Absent entries are
    /// treated as clean. Directories are pre-populated with `Modified` when any
    /// descendant has changes (see `fs::git_status::collect`).
    pub git_status: HashMap<PathBuf, GitFileStatus>,
    /// `true` once the tree has been aligned to the open file at least once via
    /// [`reveal_path`]. Latches permanently so watcher-triggered rediscoveries
    /// never re-steal the cursor back to the open file (#17) — even across a
    /// transient empty state (all files deleted then recreated), which a
    /// "nothing selected yet" check would misread as a fresh first discovery.
    pub aligned: bool,
}

/// A single visible row in the flattened file-tree list.
#[derive(Debug, Clone)]
pub struct FlatItem {
    /// Absolute path to the file or directory.
    pub path: PathBuf,
    /// Display name (file-name component only).
    pub name: String,
    /// `true` if this entry represents a directory.
    pub is_dir: bool,
    /// Visual indent level (0 = root children).
    pub depth: usize,
}

impl FileTreeState {
    /// Replace the entry tree and rebuild the flat list, preserving the
    /// selection by path.
    ///
    /// The watcher fires `rebuild` on every filesystem change, so the cursor
    /// must follow the *same item* across a rebuild rather than a fixed index —
    /// otherwise an added/removed sibling row would shift the highlight under
    /// the user (and on noisy filesystems re-running this every second would
    /// make the tree unusable, see #17). When the previously selected path is
    /// gone, the old index is clamped into range so the cursor stays near where
    /// it was; an empty tree clears the selection. When there was no previous
    /// selection (the first populate), the cursor lands on the first row —
    /// callers that need the cursor to track a specific path on first populate
    /// follow up with [`reveal_path`].
    pub fn rebuild(&mut self, entries: Vec<FileEntry>) {
        let prev_path = self.selected_path().map(|p| p.to_path_buf());
        let prev_idx = self.list_state.selected();
        self.entries = entries;
        self.flatten_visible();

        if self.flat_items.is_empty() {
            self.list_state.select(None);
            return;
        }

        // `flat_items` is non-empty here (guarded by the early return above), so
        // `len() - 1` cannot underflow.
        let restored = prev_path
            .as_deref()
            .and_then(|p| self.flat_items.iter().position(|item| item.path == p))
            .or_else(|| prev_idx.map(|i| i.min(self.flat_items.len() - 1)))
            .unwrap_or(0);
        self.list_state.select(Some(restored));
    }

    /// Rebuild `flat_items` from the current `entries` and `expanded` set.
    ///
    /// Uses `std::mem::take` to avoid cloning the entire entry tree: the
    /// entries are temporarily moved out, flattened via a standalone function,
    /// then moved back — no allocation beyond the flat list itself.
    pub fn flatten_visible(&mut self) {
        self.flat_items.clear();
        // Take entries out to satisfy the borrow checker (we need &self.expanded
        // and &mut self.flat_items simultaneously).
        let entries = std::mem::take(&mut self.entries);
        flatten_entries(&entries, &self.expanded, 0, &mut self.flat_items);
        self.entries = entries;
    }

    /// Return the path of the currently selected item, if any.
    pub fn selected_path(&self) -> Option<&std::path::Path> {
        let idx = self.list_state.selected()?;
        self.flat_items.get(idx).map(|item| item.path.as_path())
    }

    /// Return a reference to the currently selected flat item, if any.
    pub fn selected_item(&self) -> Option<&FlatItem> {
        let idx = self.list_state.selected()?;
        self.flat_items.get(idx)
    }

    /// Move the cursor up one row, clamping at the top.
    pub fn move_up(&mut self) {
        if self.flat_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Move the cursor down one row, clamping at the bottom.
    pub fn move_down(&mut self) {
        if self.flat_items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i < self.flat_items.len() - 1 => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Toggle the expansion state of the selected directory, then re-flatten.
    pub fn toggle_expand(&mut self) {
        if let Some(item) = self.selected_item().cloned()
            && item.is_dir
        {
            if self.expanded.contains(&item.path) {
                self.expanded.remove(&item.path);
            } else {
                self.expanded.insert(item.path);
            }
            self.flatten_visible();
        }
    }

    /// Move the cursor to the first item.
    pub fn go_first(&mut self) {
        if !self.flat_items.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Move the cursor to the last item.
    pub fn go_last(&mut self) {
        if !self.flat_items.is_empty() {
            self.list_state.select(Some(self.flat_items.len() - 1));
        }
    }

    /// Expand every ancestor directory of `path`, rebuild the flat item list,
    /// and select the row for `path` if present.
    ///
    /// Safe to call on paths that are outside the tree root — the walk simply
    /// finds no matching ancestors and no matching row, making this a no-op.
    /// Intended to be invoked whenever a file is opened programmatically (search
    /// result, link pick, session restore, etc.) so the tree is always aligned
    /// with the viewer.
    pub fn reveal_path(&mut self, path: &Path) {
        // Walk up the directory hierarchy, inserting each ancestor into the
        // expanded set.  We stop when `parent()` returns `None` (hit filesystem
        // root) or when the parent equals the path itself (POSIX root "/" is its
        // own parent — guards against infinite loops).
        let mut cursor = path.parent();
        while let Some(parent) = cursor {
            self.expanded.insert(parent.to_path_buf());
            let next = parent.parent();
            // POSIX root "/" has itself as its own parent; stop to avoid a loop.
            if next == Some(parent) {
                break;
            }
            cursor = next;
        }
        self.flatten_visible();
        if let Some(idx) = self.flat_items.iter().position(|item| item.path == path) {
            self.list_state.select(Some(idx));
            // Record that the tree has been aligned to a file at least once, so
            // passive watcher rediscoveries stop forcing a realignment (#17).
            self.aligned = true;
        }
    }
}

/// Recursively walk `entries` and append visible rows to `out`.
///
/// A directory's children are only appended when the directory's path is
/// present in `expanded`. This is a free function (not a method) so that
/// `entries` can be borrowed immutably while `out` is built mutably without
/// conflicting with the surrounding `FileTreeState` borrow.
fn flatten_entries(
    entries: &[FileEntry],
    expanded: &HashSet<PathBuf>,
    depth: usize,
    out: &mut Vec<FlatItem>,
) {
    for entry in entries {
        out.push(FlatItem {
            path: entry.path.clone(),
            name: entry.name.clone(),
            is_dir: entry.is_dir,
            depth,
        });

        if entry.is_dir && expanded.contains(&entry.path) {
            flatten_entries(&entry.children, expanded, depth + 1, out);
        }
    }
}

/// Render the file-tree panel into `area`.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let p = &app.palette;

    let border_style = if focused {
        p.border_focused_style()
    } else {
        p.border_style()
    };

    let block = Block::default()
        .title(" Files ")
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(p.background));

    let items: Vec<ListItem> = app
        .tree
        .flat_items
        .iter()
        .map(|item| {
            // One space per depth level keeps the tree readable while
            // maximising filename width on deep structures.
            let indent = " ".repeat(item.depth);
            let (prefix, prefix_color) = if item.is_dir {
                let marker = if app.tree.expanded.contains(&item.path) {
                    "▾ "
                } else {
                    "▸ "
                };
                (marker, p.accent)
            } else {
                ("  ", p.foreground)
            };

            let name_color: Color = match app.tree.git_status.get(&item.path) {
                Some(GitFileStatus::New) => p.git_new,
                Some(GitFileStatus::Modified) => p.git_modified,
                None => {
                    if item.is_dir {
                        p.accent
                    } else {
                        p.foreground
                    }
                }
            };

            let prefix_style = Style::default()
                .fg(prefix_color)
                .add_modifier(if item.is_dir {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
            let name_style = Style::default()
                .fg(name_color)
                .add_modifier(if item.is_dir {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });

            let line = Line::from(vec![
                Span::styled(format!("{indent}{prefix}"), prefix_style),
                Span::styled(item.name.clone(), name_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(p.selected_style())
        .highlight_symbol("│ ");

    f.render_stateful_widget(list, area, &mut app.tree.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::discovery::FileEntry;

    /// Build a small synthetic tree:
    ///
    /// ```
    /// deep/        (dir)
    ///   nested/    (dir)
    ///     file.md  (file)
    /// other.md     (file)
    /// ```
    fn make_test_tree() -> (FileTreeState, PathBuf) {
        let deep_nested_file = PathBuf::from("/root/deep/nested/file.md");
        let deep = PathBuf::from("/root/deep");
        let nested = PathBuf::from("/root/deep/nested");

        let entries = vec![
            FileEntry {
                path: deep.clone(),
                name: "deep".to_string(),
                is_dir: true,
                children: vec![FileEntry {
                    path: nested,
                    name: "nested".to_string(),
                    is_dir: true,
                    children: vec![FileEntry {
                        path: deep_nested_file.clone(),
                        name: "file.md".to_string(),
                        is_dir: false,
                        children: vec![],
                    }],
                }],
            },
            FileEntry {
                path: PathBuf::from("/root/other.md"),
                name: "other.md".to_string(),
                is_dir: false,
                children: vec![],
            },
        ];

        let mut state = FileTreeState::default();
        state.rebuild(entries);
        (state, deep_nested_file)
    }

    /// `reveal_path` must expand all ancestor directories and select the target.
    #[test]
    fn reveal_path_expands_ancestors() {
        let (mut state, target) = make_test_tree();
        // Initially only the top-level items are visible (no dirs expanded).
        let initial_len = state.flat_items.len();
        assert_eq!(initial_len, 2, "only deep/ and other.md at root");

        state.reveal_path(&target);

        // Both ancestor directories must now be in the expanded set.
        assert!(
            state.expanded.contains(Path::new("/root/deep")),
            "deep/ should be expanded"
        );
        assert!(
            state.expanded.contains(Path::new("/root/deep/nested")),
            "deep/nested/ should be expanded"
        );

        // The flat list must now contain all 4 entries.
        assert_eq!(state.flat_items.len(), 4);

        // The selection must point at the target file.
        let selected = state.list_state.selected().expect("a row must be selected");
        assert_eq!(state.flat_items[selected].path, target);
    }

    /// Calling `reveal_path` on a path not present in the tree must be a no-op.
    #[test]
    fn reveal_path_on_unknown_file_is_noop() {
        let (mut state, _) = make_test_tree();
        let before_len = state.flat_items.len();
        let before_sel = state.list_state.selected();

        state.reveal_path(Path::new("/nonexistent/path/file.md"));

        // The flat list shape is unchanged (ancestors inserted into `expanded` but
        // they don't match real tree nodes, so flatten produces the same output).
        assert_eq!(state.flat_items.len(), before_len);
        // Selection is unchanged because the file isn't in the list.
        assert_eq!(state.list_state.selected(), before_sel);
    }

    /// Calling `reveal_path` twice with the same path is idempotent.
    #[test]
    fn reveal_path_idempotent() {
        let (mut state, target) = make_test_tree();

        state.reveal_path(&target);
        let len_after_first = state.flat_items.len();
        let sel_after_first = state.list_state.selected();

        state.reveal_path(&target);

        assert_eq!(state.flat_items.len(), len_after_first);
        assert_eq!(state.list_state.selected(), sel_after_first);
    }

    /// Build a flat list of N sibling files `/root/0.md`, `/root/1.md`, …
    fn flat_files(n: usize) -> Vec<FileEntry> {
        (0..n)
            .map(|i| FileEntry {
                path: PathBuf::from(format!("/root/{i}.md")),
                name: format!("{i}.md"),
                is_dir: false,
                children: vec![],
            })
            .collect()
    }

    /// `rebuild` must preserve the selection by *path*, not by index: when a
    /// sibling above the selected file is removed, the cursor follows the file.
    #[test]
    fn rebuild_follows_selected_path_when_siblings_shift() {
        let mut state = FileTreeState::default();
        state.rebuild(flat_files(3)); // 0.md, 1.md, 2.md
        state.list_state.select(Some(2)); // select 2.md
        assert_eq!(state.selected_path(), Some(Path::new("/root/2.md")));

        // Drop 0.md — 2.md moves from index 2 to index 1.
        state.rebuild(vec![
            FileEntry {
                path: PathBuf::from("/root/1.md"),
                name: "1.md".into(),
                is_dir: false,
                children: vec![],
            },
            FileEntry {
                path: PathBuf::from("/root/2.md"),
                name: "2.md".into(),
                is_dir: false,
                children: vec![],
            },
        ]);

        assert_eq!(
            state.selected_path(),
            Some(Path::new("/root/2.md")),
            "selection must track the path, not the old index"
        );
        assert_eq!(state.list_state.selected(), Some(1));
    }

    /// When the selected file is deleted, the cursor clamps to the last row
    /// rather than jumping to the top or panicking.
    #[test]
    fn rebuild_clamps_to_last_row_when_selected_path_gone() {
        let mut state = FileTreeState::default();
        state.rebuild(flat_files(3)); // 0.md, 1.md, 2.md
        state.list_state.select(Some(2)); // select the last row, 2.md

        // Rebuild without 2.md — the selected path is now gone.
        state.rebuild(flat_files(2)); // 0.md, 1.md

        assert_eq!(
            state.list_state.selected(),
            Some(1),
            "cursor must clamp to the new last row, not reset to 0"
        );
        assert_eq!(state.selected_path(), Some(Path::new("/root/1.md")));
    }

    /// Rebuilding with no entries clears the selection entirely.
    #[test]
    fn rebuild_empty_entries_clears_selection() {
        let mut state = FileTreeState::default();
        state.rebuild(flat_files(2));
        assert!(
            state.list_state.selected().is_some(),
            "precondition: a row is selected"
        );

        state.rebuild(vec![]);

        assert_eq!(
            state.list_state.selected(),
            None,
            "empty rebuild must clear the selection"
        );
        assert!(state.flat_items.is_empty());
        assert_eq!(state.selected_path(), None);
    }

    /// A deep selection inside an expanded directory survives a `rebuild` with
    /// the same entries — the `expanded` set is the mechanism that lets
    /// selection-by-path find the nested row again after a watcher refresh.
    #[test]
    fn rebuild_preserves_expanded_and_deep_selection() {
        let (mut state, deep_file) = make_test_tree();
        state.reveal_path(&deep_file); // expand ancestors + select the nested file
        assert_eq!(state.selected_path(), Some(deep_file.as_path()));
        let expanded_before = state.expanded.clone();

        // Re-run discovery with the identical tree (simulates a watcher event).
        let (rebuilt, _) = make_test_tree();
        state.rebuild(rebuilt.entries);

        assert_eq!(
            state.selected_path(),
            Some(deep_file.as_path()),
            "deep selection must survive a same-tree rebuild"
        );
        assert_eq!(
            state.expanded, expanded_before,
            "expanded directories must persist across rebuild"
        );
    }

    /// `reveal_path` latches `aligned` only when it actually selects a row; a
    /// reveal of an absent path must not flip the latch (so a later, real
    /// reveal can still align the tree to the open file).
    #[test]
    fn reveal_path_sets_aligned_only_on_a_real_match() {
        let (mut state, target) = make_test_tree();
        assert!(!state.aligned, "fresh tree is not yet aligned");

        state.reveal_path(Path::new("/nonexistent/x.md"));
        assert!(
            !state.aligned,
            "reveal of an absent path must not latch aligned"
        );

        state.reveal_path(&target);
        assert!(state.aligned, "reveal of a present path must latch aligned");
    }
}
