use crate::app::App;
use crate::fs::discovery::FileEntry;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState},
};
use std::collections::HashSet;
use std::path::PathBuf;

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
            let indent = "  ".repeat(item.depth);
            let prefix = if item.is_dir {
                if app.tree.expanded.contains(&item.path) {
                    "▼ "
                } else {
                    "▶ "
                }
            } else {
                "  "
            };
            let style = if item.is_dir {
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.foreground)
            };
            ListItem::new(Line::styled(
                format!("{indent}{prefix}{}", item.name),
                style,
            ))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(p.selected_style())
        .highlight_symbol("│ ");

    f.render_stateful_widget(list, area, &mut app.tree.list_state);
}
