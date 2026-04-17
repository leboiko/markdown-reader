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
    /// Replace the entry tree and rebuild the flat list, preserving selection.
    pub fn rebuild(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.flatten_visible();
        if !self.flat_items.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
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
}
