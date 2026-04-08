use crate::action::Action;
use notify_debouncer_mini::{
    DebounceEventResult, DebouncedEventKind, Debouncer, new_debouncer, notify::RecommendedWatcher,
};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

/// Spawn a filesystem watcher that sends [`Action::FilesChanged`] whenever a
/// markdown file under `root` changes on disk.
///
/// The returned [`Debouncer`] must be kept alive for as long as watching is
/// desired; dropping it stops the watcher.
///
/// # Arguments
///
/// * `root` - The directory tree to watch recursively.
/// * `tx`   - Sender used to inject [`Action::FilesChanged`] into the app loop.
///
/// # Errors
///
/// Returns an error if the OS watcher cannot be created or `root` cannot be
/// registered for watching.
pub fn spawn_watcher(
    root: &Path,
    tx: mpsc::UnboundedSender<Action>,
) -> anyhow::Result<Debouncer<RecommendedWatcher>> {
    // notify-debouncer-mini 0.7 uses a callback instead of a channel sender.
    // The closure is called from a background thread with a batch of debounced events.
    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let changed: Vec<_> = events
                .iter()
                .filter(|e| {
                    e.kind == DebouncedEventKind::Any
                        && e.path
                            .extension()
                            .is_some_and(|ext| ext == "md" || ext == "markdown")
                })
                .map(|e| e.path.clone())
                .collect();

            if !changed.is_empty() {
                let _ = tx.send(Action::FilesChanged(changed));
            }
        },
    )?;

    debouncer.watcher().watch(
        root,
        notify_debouncer_mini::notify::RecursiveMode::Recursive,
    )?;

    Ok(debouncer)
}
