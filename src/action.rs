use crate::markdown::MermaidBlockId;
use crate::mermaid::MermaidEntry;
use crossterm::event::{KeyEvent, MouseEvent};
use std::path::PathBuf;

/// All actions that can be dispatched through the application event loop.
///
/// `RawKey` events are produced by the input task and translated into more
/// specific variants by the focused-widget key handlers. Many variants are
/// constructed only through `handle_key` dispatch paths, so dead-code analysis
/// produces false positives for this enum.
#[allow(dead_code)]
pub enum Action {
    /// Exit the application.
    Quit,

    /// Raw terminal key event — mapped to a concrete action based on focus.
    RawKey(KeyEvent),

    /// Move focus to the file tree panel.
    FocusLeft,
    /// Move focus to the viewer panel.
    FocusRight,

    /// Move the tree cursor up one entry.
    TreeUp,
    /// Move the tree cursor down one entry.
    TreeDown,
    /// Toggle expansion of the selected directory.
    TreeToggle,
    /// Open the selected file or toggle the selected directory.
    TreeSelect,
    /// Jump to the first entry in the tree.
    TreeFirst,
    /// Jump to the last entry in the tree.
    TreeLast,

    /// Scroll the viewer up by `n` lines.
    ScrollUp(u16),
    /// Scroll the viewer down by `n` lines.
    ScrollDown(u16),
    /// Scroll the viewer up by half a page.
    ScrollHalfPageUp,
    /// Scroll the viewer down by half a page.
    ScrollHalfPageDown,
    /// Jump to the top of the document.
    ScrollToTop,
    /// Jump to the bottom of the document.
    ScrollToBottom,

    /// Open the search bar and reset its state.
    EnterSearch,
    /// Close the search bar and return focus to the tree.
    ExitSearch,
    /// Append a character to the search query.
    SearchInput(char),
    /// Remove the last character from the search query.
    SearchBackspace,
    /// Select the next search result.
    SearchNext,
    /// Select the previous search result.
    SearchPrev,
    /// Confirm the current search result and open the file.
    SearchConfirm,
    /// Toggle between file-name and content search modes.
    SearchToggleMode,

    /// Notify the app that one or more watched files changed on disk.
    FilesChanged(Vec<PathBuf>),

    /// Terminal was resized to the given (width, height).
    Resize(u16, u16),

    /// Raw mouse event forwarded from crossterm.
    Mouse(MouseEvent),

    /// A background mermaid render completed; entry is ready to be stored.
    ///
    /// Boxed to avoid inflating every `Action` variant by the size of `MermaidEntry`.
    MermaidReady(MermaidBlockId, Box<MermaidEntry>),
}
