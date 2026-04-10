use crate::app::DocSearchState;
use crate::theme::Palette;
use crate::ui::markdown_view::MarkdownViewState;
use std::path::PathBuf;

/// Maximum number of tabs that can be open simultaneously.
pub const MAX_TABS: usize = 32;

/// Opaque stable identifier for a tab.
///
/// Uses a monotonically increasing counter on [`Tabs`] so the id is stable
/// across insertions and removals (unlike a bare index, which shifts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabId(pub u32);

/// All per-document state owned by a single tab.
pub struct Tab {
    pub id: TabId,
    pub view: MarkdownViewState,
    /// In-document find state is document-specific and travels with the tab.
    pub doc_search: DocSearchState,
}

/// Ordered collection of open tabs with an active-tab pointer.
///
/// `view_height` is a viewport property shared across all tabs; it is updated
/// once per draw call and used by every tab's scroll methods.
pub struct Tabs {
    pub tabs: Vec<Tab>,
    /// The currently visible tab.
    pub active: Option<TabId>,
    /// The previously active tab, used for backtick (`` ` ``) navigation.
    pub previous: Option<TabId>,
    next_id: u32,
    /// Inner height of the viewer panel (rows minus borders), updated each draw.
    pub view_height: u32,
}

impl Tabs {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            previous: None,
            next_id: 0,
            view_height: 0,
        }
    }

    fn alloc_id(&mut self) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        id
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Return a shared reference to the active tab, if any.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| {
            let idx = self.index_of(id)?;
            self.tabs.get(idx)
        })
    }

    /// Return a mutable reference to the active tab, if any.
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active?;
        let idx = self.index_of(id)?;
        self.tabs.get_mut(idx)
    }

    /// Return the 0-based index of the active tab in the `tabs` slice.
    pub fn active_index(&self) -> Option<usize> {
        self.active.and_then(|id| self.index_of(id))
    }

    pub fn set_active(&mut self, id: TabId) {
        if self.active != Some(id) {
            self.previous = self.active;
            self.active = Some(id);
        }
    }

    /// Open or focus a tab for `path`.
    ///
    /// Behavior:
    /// - If `path` is already open, activate that tab and return its id.
    /// - If `new_tab == false` and there is an active tab, replace it.
    /// - If `tabs.len() >= MAX_TABS`, silently refuse and return current active.
    /// - Otherwise push a new tab, activate it, and return its id.
    ///
    /// This method does **not** do filesystem I/O; the caller is responsible
    /// for calling [`MarkdownViewState::load`] when a new tab was created.
    pub fn open_or_focus(&mut self, path: &PathBuf, new_tab: bool) -> (TabId, OpenOutcome) {
        // Deduplicate: if already open, just switch.
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|t| t.view.current_path.as_ref() == Some(path))
        {
            let id = existing.id;
            self.set_active(id);
            return (id, OpenOutcome::Focused);
        }

        // Replace active tab when requested (no new tab).
        if !new_tab && let Some(id) = self.active {
            return (id, OpenOutcome::Replaced);
        }

        // Enforce cap.
        if self.tabs.len() >= MAX_TABS {
            let fallback = self.active.unwrap_or(TabId(0));
            return (fallback, OpenOutcome::Capped);
        }

        // Push new tab.
        let id = self.alloc_id();
        self.tabs.push(Tab {
            id,
            view: MarkdownViewState::default(),
            doc_search: DocSearchState::default(),
        });
        self.set_active(id);
        (id, OpenOutcome::Opened)
    }

    /// Close the tab with `id`. Returns `true` if the tab was found and removed.
    ///
    /// After closing, the active tab is updated: previous tab if it still
    /// exists, otherwise the neighbour at the same index (clamped), or `None`.
    pub fn close(&mut self, id: TabId) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        self.tabs.remove(idx);

        if self.tabs.is_empty() {
            self.active = None;
            self.previous = None;
            return true;
        }

        if let Some(prev) = self.previous
            && prev != id
            && self.index_of(prev).is_some()
        {
            self.previous = None;
            self.active = Some(prev);
        } else {
            let new_idx = idx.min(self.tabs.len() - 1);
            self.active = Some(self.tabs[new_idx].id);
            self.previous = None;
        }
        true
    }

    /// Return the number of open tabs.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Activate the tab after the current one, wrapping around.
    pub fn next(&mut self) {
        let Some(idx) = self.active_index() else {
            return;
        };
        let next_idx = (idx + 1) % self.tabs.len();
        let id = self.tabs[next_idx].id;
        self.set_active(id);
    }

    /// Activate the tab before the current one, wrapping around.
    pub fn prev(&mut self) {
        let Some(idx) = self.active_index() else {
            return;
        };
        let prev_idx = if idx == 0 {
            self.tabs.len() - 1
        } else {
            idx - 1
        };
        let id = self.tabs[prev_idx].id;
        self.set_active(id);
    }

    /// Activate a tab by 1-based index. Out-of-range is a silent no-op.
    pub fn activate_by_index(&mut self, one_based: usize) {
        if one_based == 0 || one_based > self.tabs.len() {
            return;
        }
        let id = self.tabs[one_based - 1].id;
        self.set_active(id);
    }

    /// Jump to the last tab.
    pub fn activate_last(&mut self) {
        if let Some(last) = self.tabs.last() {
            let id = last.id;
            self.set_active(id);
        }
    }

    /// Activate the previously active tab (backtick navigation).
    pub fn activate_previous(&mut self) {
        let Some(prev) = self.previous else {
            return;
        };
        if self.index_of(prev).is_none() {
            self.previous = None;
            return;
        }
        let current = self.active;
        self.active = Some(prev);
        self.previous = current;
    }

    /// Re-render every open tab with the given palette, preserving scroll offsets.
    pub fn rerender_all(&mut self, palette: &Palette) {
        for tab in &mut self.tabs {
            if tab.view.current_path.is_some() {
                let content = tab.view.content.clone();
                let path = tab.view.current_path.clone().unwrap();
                let name = tab.view.file_name.clone();
                let scroll = tab.view.scroll_offset;
                tab.view.load(path, name, content, palette);
                tab.view.scroll_offset = scroll.min(tab.view.total_lines.saturating_sub(1));
            }
        }
    }
}

/// The outcome of a [`Tabs::open_or_focus`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOutcome {
    /// An existing tab was focused (deduplicated).
    Focused,
    /// The active tab's content was replaced.
    Replaced,
    /// A new tab was pushed and activated.
    Opened,
    /// The tab cap was reached; nothing changed.
    Capped,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/fake/{name}"))
    }

    /// Open `name` in `tabs`, simulating the caller loading content.
    fn open(tabs: &mut Tabs, name: &str, new_tab: bool) -> (TabId, OpenOutcome) {
        let path = make_path(name);
        let (id, outcome) = tabs.open_or_focus(&path, new_tab);
        if matches!(outcome, OpenOutcome::Opened | OpenOutcome::Replaced) {
            let tab = tabs.active_tab_mut().unwrap();
            tab.view.current_path = Some(path);
            tab.view.file_name = name.to_string();
        }
        (id, outcome)
    }

    #[test]
    fn open_or_focus_creates_new_tab() {
        let mut tabs = Tabs::new();
        let (_, outcome) = open(&mut tabs, "a.md", true);
        assert_eq!(outcome, OpenOutcome::Opened);
        assert_eq!(tabs.len(), 1);
        assert!(tabs.active.is_some());
    }

    #[test]
    fn open_or_focus_dedupes_by_path() {
        let mut tabs = Tabs::new();
        open(&mut tabs, "a.md", true);
        let (_, outcome) = open(&mut tabs, "a.md", true);
        assert_eq!(outcome, OpenOutcome::Focused);
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn open_or_focus_replaces_active_when_new_tab_false() {
        let mut tabs = Tabs::new();
        open(&mut tabs, "a.md", true);
        let (_, outcome) = open(&mut tabs, "b.md", false);
        assert_eq!(outcome, OpenOutcome::Replaced);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_tab().unwrap().view.file_name, "b.md");
    }

    #[test]
    fn open_or_focus_pushes_when_new_tab_true() {
        let mut tabs = Tabs::new();
        open(&mut tabs, "a.md", true);
        let (_, outcome) = open(&mut tabs, "b.md", true);
        assert_eq!(outcome, OpenOutcome::Opened);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_tab().unwrap().view.file_name, "b.md");
    }

    #[test]
    fn open_or_focus_caps_at_32() {
        let mut tabs = Tabs::new();
        for i in 0..MAX_TABS {
            open(&mut tabs, &format!("{i}.md"), true);
        }
        assert_eq!(tabs.len(), MAX_TABS);
        let (_, outcome) = open(&mut tabs, "overflow.md", true);
        assert_eq!(outcome, OpenOutcome::Capped);
        assert_eq!(tabs.len(), MAX_TABS);
    }

    #[test]
    fn close_active_last_tab() {
        let mut tabs = Tabs::new();
        let (id, _) = open(&mut tabs, "a.md", true);
        let removed = tabs.close(id);
        assert!(removed);
        assert_eq!(tabs.len(), 0);
        assert!(tabs.active.is_none());
    }

    #[test]
    fn close_active_switches_to_most_recent() {
        let mut tabs = Tabs::new();
        open(&mut tabs, "a.md", true);
        let (b_id, _) = open(&mut tabs, "b.md", true);
        let (c_id, _) = open(&mut tabs, "c.md", true);
        tabs.close(c_id);
        assert_eq!(tabs.active, Some(b_id));
    }

    #[test]
    fn next_prev_wraparound() {
        let mut tabs = Tabs::new();
        let (a_id, _) = open(&mut tabs, "a.md", true);
        open(&mut tabs, "b.md", true);
        let (c_id, _) = open(&mut tabs, "c.md", true);
        // Active is C (index 2), next wraps to A (index 0).
        tabs.next();
        assert_eq!(tabs.active, Some(a_id));
        // Active is A (index 0), prev wraps to C (index 2).
        tabs.prev();
        assert_eq!(tabs.active, Some(c_id));
    }

    #[test]
    fn activate_previous_roundtrip() {
        let mut tabs = Tabs::new();
        open(&mut tabs, "a.md", true);
        let (b_id, _) = open(&mut tabs, "b.md", true);
        let (c_id, _) = open(&mut tabs, "c.md", true);
        // active=C, previous=B
        tabs.activate_previous(); // now active=B, previous=C
        assert_eq!(tabs.active, Some(b_id));
        tabs.activate_previous(); // now active=C, previous=B
        assert_eq!(tabs.active, Some(c_id));
    }

    #[test]
    fn activate_by_index_bounds() {
        let mut tabs = Tabs::new();
        let (a_id, _) = open(&mut tabs, "a.md", true);
        open(&mut tabs, "b.md", true);
        // Out-of-range: no-op.
        tabs.activate_by_index(0);
        tabs.activate_by_index(99);
        // Activate first tab by 1-based index 1.
        tabs.activate_by_index(1);
        assert_eq!(tabs.active, Some(a_id));
    }
}
