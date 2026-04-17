mod action;
mod app;
mod config;
mod event;
mod fs;
mod markdown;
mod mermaid;
mod state;
mod theme;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    cursor::Show,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::path::PathBuf;

/// Holds a reference to stdout so the terminal can be restored on drop.
///
/// By constructing this guard _after_ entering raw mode / alternate screen,
/// we guarantee that `disable_raw_mode`, `LeaveAlternateScreen`,
/// `DisableMouseCapture`, and `show_cursor` are called even if the app
/// panics or returns an error mid-run.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            Show,
        );
    }
}

#[derive(Parser, Debug)]
#[command(name = "markdown-reader", about = "A TUI markdown file viewer")]
struct Cli {
    /// Path to browse: a directory opens the tree at that root; a file opens
    /// the tree at its parent directory and immediately displays the file.
    /// Defaults to the current directory.
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve symlinks and relative components so all path comparisons inside
    // the app use the same canonical form.
    let canonical = cli.path.canonicalize()?;

    // When the user passes a file, root the tree at its parent directory and
    // remember the file so the event loop can open it once action_tx is ready.
    // When the path is a directory (the common case) there is no initial file.
    let (root, initial_file) = if canonical.is_file() {
        let parent = canonical
            .parent()
            // A file always has a parent (at minimum "/"), so this is only None
            // for the filesystem root itself, which cannot be a regular file.
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        (parent, Some(canonical))
    } else {
        (canonical, None)
    };

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    App::new(root, initial_file).run(&mut terminal).await
}
