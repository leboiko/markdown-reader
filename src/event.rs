use crate::action::Action;
use crossterm::event::{self, Event, MouseEventKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// Convert a `crossterm::Event` into an [`Action`], or `None` if the event
/// should be silently dropped at the input boundary.
///
/// **Mouse-motion filter (idle-CPU fix):** with `EnableMouseCapture` on,
/// many terminals (Ghostty, Kitty, iTerm2, modern xterm with SGR mode)
/// emit a `MouseEventKind::Moved` event for every cell the cursor crosses
/// over the terminal area — even when no button is pressed. The main loop
/// at `App::run` redraws on every action, so an unfiltered motion stream
/// produced a continuous `move → action → wake → full redraw → move → …`
/// tight loop that consumed all of one CPU core when the user's cursor
/// passed over the terminal window. No handler in this codebase reads
/// `MouseEventKind::Moved`, so dropping it at the input boundary is safe.
///
/// `Drag` events are kept (drag-select / scrollbar-drag use cases). Click,
/// scroll, and resize events are kept.
pub(crate) fn event_to_action(evt: Event) -> Option<Action> {
    match evt {
        // Only handle key-down events. On Windows, crossterm emits both
        // Press and Release for every keystroke; forwarding both would
        // duplicate every action.
        Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
            Some(Action::RawKey(key))
        }
        Event::Resize(w, h) => Some(Action::Resize(w, h)),
        // Drop hover-motion events at the input boundary — no handler
        // consumes them and they otherwise drive the main loop into a
        // continuous redraw.
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved) => None,
        Event::Mouse(m) => Some(Action::Mouse(m)),
        _ => None,
    }
}

/// Drives the terminal event loop and forwards actions to the application.
///
/// `EventHandler` owns the receiving end of an unbounded channel. A background
/// thread polls crossterm for keyboard and resize events and sends them as
/// [`Action`] values into the channel.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Action>,
    /// Shared flag to signal the background thread to stop.
    stop: Arc<AtomicBool>,
}

impl EventHandler {
    /// Create a new handler and spawn the background input-polling thread.
    ///
    /// # Returns
    ///
    /// A tuple of `(EventHandler, UnboundedSender<Action>)`. The sender is
    /// shared with the rest of the application so that non-input sources (e.g.
    /// the filesystem watcher) can also inject actions into the event loop.
    pub fn new() -> (Self, mpsc::UnboundedSender<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();

        // Use a real OS thread — crossterm::event::poll is blocking I/O and
        // must not run on the tokio async executor.
        std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                if event::poll(Duration::from_millis(50)).unwrap_or(false)
                    && let Ok(evt) = event::read()
                    && let Some(action) = event_to_action(evt)
                    && event_tx.send(action).is_err()
                {
                    break;
                }
            }
        });

        (Self { rx, stop }, tx)
    }

    /// Wait for the next action from any source.
    ///
    /// Returns `None` when every sender has been dropped and the channel is
    /// empty, which signals that the application should shut down.
    pub async fn next(&mut self) -> Option<Action> {
        self.rx.recv().await
    }
}

impl Drop for EventHandler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent};

    /// Hover-motion mouse events MUST be dropped at the input boundary so
    /// the main render loop doesn't wake on every cursor movement when
    /// `EnableMouseCapture` is on. A no-op fix that returns `Some(Action::Mouse)`
    /// here would re-introduce the idle-CPU regression.
    #[test]
    fn mouse_moved_events_are_dropped() {
        for (col, row) in [(0u16, 0u16), (10, 5), (100, 50)] {
            let evt = Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: col,
                row,
                modifiers: KeyModifiers::empty(),
            });
            assert!(
                event_to_action(evt).is_none(),
                "Moved at ({col},{row}) must produce None — got Some"
            );
        }
    }

    /// Click, scroll, and drag mouse events MUST still pass through —
    /// dropping all mouse events would break click-to-focus, scroll
    /// navigation, and drag-select.
    #[test]
    fn non_motion_mouse_events_pass_through() {
        let cases = [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::ScrollUp,
            MouseEventKind::ScrollDown,
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ];
        for kind in cases {
            let evt = Event::Mouse(MouseEvent {
                kind,
                column: 5,
                row: 5,
                modifiers: KeyModifiers::empty(),
            });
            let action = event_to_action(evt);
            assert!(
                matches!(action, Some(Action::Mouse(_))),
                "non-motion mouse event {kind:?} must produce Some(Action::Mouse) — got something else"
            );
        }
    }

    /// Key-press events still pass through; key-release events are dropped
    /// (Windows emits both for every keystroke).
    #[test]
    fn key_press_passes_release_drops() {
        let press = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
            KeyEventKind::Press,
        ));
        assert!(matches!(event_to_action(press), Some(Action::RawKey(_))));

        let release = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
            KeyEventKind::Release,
        ));
        assert!(event_to_action(release).is_none());
    }

    /// Resize events pass through unchanged.
    #[test]
    fn resize_events_pass_through() {
        let evt = Event::Resize(80, 24);
        assert!(matches!(event_to_action(evt), Some(Action::Resize(80, 24))));
    }
}
