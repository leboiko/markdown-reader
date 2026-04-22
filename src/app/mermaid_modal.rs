/// Mermaid-modal open + key handling, mirroring `table_modal.rs`.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;

impl App {
    /// Open the mermaid modal if the block at (or nearest to) the cursor is a
    /// mermaid block.
    ///
    /// Resolution order mirrors [`Self::try_open_table_modal`]:
    /// 1. The mermaid block whose row range contains the cursor.
    /// 2. Otherwise, the first mermaid block intersecting the viewport.
    ///
    /// `block_id` + `source` are captured so the renderer can re-look-up
    /// the cache state on every frame (live updates while the modal is
    /// open, e.g. when a queued image render finishes).
    pub(super) fn try_open_mermaid_modal(&mut self) {
        let view_height = self.tabs.view_height;
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let viewport_start = tab.view.scroll_offset;
        let viewport_end = viewport_start + view_height;
        let cursor_line = tab.view.cursor_line;

        let mut cursor_match: Option<(crate::markdown::MermaidBlockId, &str)> = None;
        let mut viewport_match: Option<(crate::markdown::MermaidBlockId, &str)> = None;
        let mut block_start = 0u32;
        for doc_block in &tab.view.rendered {
            let block_end = block_start + doc_block.height();
            if let crate::markdown::DocBlock::Mermaid { id, source, .. } = doc_block {
                if cursor_line >= block_start && cursor_line < block_end {
                    cursor_match = Some((*id, source.as_str()));
                    break;
                }
                if viewport_match.is_none()
                    && block_end > viewport_start
                    && block_start < viewport_end
                {
                    viewport_match = Some((*id, source.as_str()));
                }
            }
            block_start = block_end;
            if block_start >= viewport_end && cursor_match.is_none() && cursor_line < block_start {
                // No more blocks can intersect the viewport AND we've
                // passed the cursor line — nothing left to find.
                break;
            }
        }

        let Some((block_id, source)) = cursor_match.or(viewport_match) else {
            return;
        };
        self.mermaid_modal = Some(MermaidModalState {
            tab_id: tab.id,
            block_id,
            source: source.to_string(),
            h_scroll: 0,
            v_scroll: 0,
            text_zoom: 0,
        });
        self.focus = Focus::MermaidModal;
    }

    /// Handle a key press while the mermaid modal is open. Mirrors
    /// `handle_table_modal_key` so the two modals share muscle memory:
    ///
    /// - `q` / `Esc` / `Enter` — close.
    /// - `j` / `k` / arrows — scroll one row.
    /// - `h` / `l` / arrows — scroll one column.
    /// - `H` / `L` — half-page horizontal step.
    /// - `d` / `u` / `PageDown` / `PageUp` — half-page vertical step.
    /// - `g` then `g` — jump to top-left.
    /// - `G` — jump to bottom.
    /// - `0` / `$` — jump to leftmost / rightmost column.
    pub(super) fn handle_mermaid_modal_key(&mut self, code: KeyCode) {
        // `g` chord — same shape as `handle_table_modal_key`.
        if self.pending_chord.take() == Some('g')
            && code == KeyCode::Char('g')
            && let Some(s) = self.mermaid_modal.as_mut()
        {
            s.v_scroll = 0;
            s.h_scroll = 0;
            return;
        }

        let view_height = crate::cast::u16_from_u32(self.tabs.view_height);
        let inner_width = self
            .mermaid_modal_rect
            .map_or(80, |r| r.width.saturating_sub(2));

        match code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => {
                self.close_mermaid_modal();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_add(1);
                }
            }
            KeyCode::Char('H') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(inner_width / 2);
                }
            }
            KeyCode::Char('L') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_add(inner_width / 2);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_add(1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('d') | KeyCode::PageDown => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_add(view_height / 2);
                }
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(view_height / 2);
                }
            }
            KeyCode::Char('G') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    // The renderer clamps v_scroll against the actual content
                    // height each frame, so a generous value here is safe.
                    s.v_scroll = u16::MAX;
                }
            }
            KeyCode::Char('0') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = 0;
                }
            }
            KeyCode::Char('$') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = u16::MAX;
                }
            }
            KeyCode::Char('g') => {
                self.pending_chord = Some('g');
            }
            // Text-mode zoom — `+` requests a more spacious layout, `-`
            // a more compact one, `=` resets. The renderer re-runs
            // `mermaid_text::render_with_width` synchronously next frame
            // (sub-millisecond for typical diagrams), so each press is
            // visible immediately. Image-mode entries ignore zoom.
            // We accept both `+` and `=` (US/UK keyboards put `+` on
            // shift-`=`, but some users land on `=` directly), and both
            // `-` and `_`. We reset on `0` chord-style? No — `0` is
            // already taken for "scroll to leftmost", so we use the bare
            // `=` for reset and require `Shift+=` (which crossterm sends
            // as `+`) for zoom-in. `-` zooms out (more compact).
            KeyCode::Char('+') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.text_zoom = s.text_zoom.saturating_add(1);
                    s.h_scroll = 0;
                    s.v_scroll = 0;
                }
            }
            KeyCode::Char('-') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.text_zoom = s.text_zoom.saturating_sub(1);
                    s.h_scroll = 0;
                    s.v_scroll = 0;
                }
            }
            KeyCode::Char('=') => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.text_zoom = 0;
                    s.h_scroll = 0;
                    s.v_scroll = 0;
                }
            }
            _ => {}
        }
    }

    /// Mouse handling for the mermaid modal — same shape as
    /// `handle_table_modal_mouse`. Click-outside closes; scroll wheel pans.
    pub(super) fn handle_mermaid_modal_mouse(&mut self, m: crossterm::event::MouseEvent) {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        let col = m.column;
        let row = m.row;
        let inside = self.mermaid_modal_rect.is_some_and(|r| {
            col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        });

        match m.kind {
            // Click inside the modal is a no-op; click outside closes it.
            MouseEventKind::Down(MouseButton::Left) if !inside => {
                self.close_mermaid_modal();
            }
            MouseEventKind::ScrollDown if inside => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_add(3);
                }
            }
            MouseEventKind::ScrollUp if inside => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.v_scroll = s.v_scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollLeft if inside => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollRight if inside => {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    s.h_scroll = s.h_scroll.saturating_add(3);
                }
            }
            // Shift + scroll wheel = horizontal pan, matching the table modal.
            _ if inside
                && m.modifiers.contains(KeyModifiers::SHIFT)
                && matches!(
                    m.kind,
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                ) =>
            {
                if let Some(s) = self.mermaid_modal.as_mut() {
                    if matches!(m.kind, MouseEventKind::ScrollDown) {
                        s.h_scroll = s.h_scroll.saturating_add(3);
                    } else {
                        s.h_scroll = s.h_scroll.saturating_sub(3);
                    }
                }
            }
            _ => {}
        }
    }
}
