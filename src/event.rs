use crate::action::Action;
use crossterm::event::{self, Event};
use std::time::Duration;
use tokio::sync::mpsc;

/// Drives the terminal event loop and forwards actions to the application.
///
/// `EventHandler` owns the receiving end of an unbounded channel. A background
/// tokio task (spawned in [`EventHandler::new`]) polls crossterm for keyboard
/// and resize events and sends them as [`Action`] values into the channel.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Action>,
}

impl EventHandler {
    /// Create a new handler and spawn the background input-polling task.
    ///
    /// # Returns
    ///
    /// A tuple of `(EventHandler, UnboundedSender<Action>)`. The sender is
    /// shared with the rest of the application so that non-input sources (e.g.
    /// the filesystem watcher) can also inject actions into the event loop.
    ///
    /// # Notes
    ///
    /// The spawned tokio task exits automatically when the last sender is
    /// dropped (the channel's `send` returns an error).
    pub fn new() -> (Self, mpsc::UnboundedSender<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        // Spawn background task: polls crossterm and forwards key/resize events.
        tokio::spawn(async move {
            loop {
                // Use a short poll timeout so we don't block the executor thread
                // longer than necessary between ticks.
                if event::poll(Duration::from_millis(50)).unwrap_or(false)
                    && let Ok(evt) = event::read()
                {
                    let action = match evt {
                        Event::Key(key) => Some(Action::RawKey(key)),
                        Event::Resize(w, h) => Some(Action::Resize(w, h)),
                        _ => None,
                    };
                    if let Some(action) = action
                        && event_tx.send(action).is_err()
                    {
                        break;
                    }
                }
            }
        });

        (Self { rx }, tx)
    }

    /// Wait for the next action from any source.
    ///
    /// Returns `None` when every sender has been dropped and the channel is
    /// empty, which signals that the application should shut down.
    pub async fn next(&mut self) -> Option<Action> {
        self.rx.recv().await
    }
}
