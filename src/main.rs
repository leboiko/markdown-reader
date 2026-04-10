mod action;
mod app;
mod config;
mod event;
mod fs;
mod markdown;
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
    /// Root directory to browse (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.path.canonicalize()?;

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    App::new(root).run(&mut terminal).await
}
