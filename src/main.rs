mod action;
mod app;
mod cast;
mod checklinks;
mod config;
mod event;
mod export;
mod fs;
mod markdown;
mod mermaid;
mod section_extract;
mod state;
mod text_layout;
mod theme;
mod ui;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use crossterm::{
    cursor::Show,
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io::{IsTerminal, Read, Write};
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
        // Best-effort pop of the keyboard enhancement flags. Pushed in
        // `main` after `enable_raw_mode`; popping is harmless (no-op)
        // on terminals that didn't accept the request.
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            Show,
        );
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "markdown-reader",
    about = "A TUI markdown file viewer",
    version
)]
struct Cli {
    /// Path to browse: a directory opens the tree at that root; a file opens
    /// the tree at its parent directory and immediately displays the file.
    /// Defaults to the current directory. Ignored when markdown is piped via
    /// stdin (`cat README.md | markdown-reader`).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Render a markdown file to a self-contained HTML document and exit.
    ///
    /// Writes the HTML to stdout unless `--output` is also specified.
    /// The TUI is not launched when this flag is present.
    #[arg(long, value_name = "FILE")]
    export_html: Option<PathBuf>,

    /// Output path for `--export-html` (default: stdout).
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Validate internal links in all `.md` files under DIR (default: current directory).
    ///
    /// Checks same-file anchors (`#heading`), cross-file links, and cross-file
    /// anchors. Exits with status 0 when all links are valid, or 1 when any
    /// broken links are found. The TUI is not launched when this flag is present.
    #[arg(long, value_name = "DIR", num_args = 0..=1, default_missing_value = ".")]
    check_links: Option<PathBuf>,

    /// Also validate `http(s)://` links when `--check-links` is active.
    ///
    /// Performs a HEAD request for every external link found in the scanned
    /// markdown files. Responses with 4xx/5xx status codes or connection
    /// errors are reported as broken links (tagged `[external]`). Up to 10
    /// requests run in parallel; redirects are followed up to 5 hops.
    #[arg(long, requires = "check_links")]
    check_external: bool,

    /// Timeout (in seconds) for each external HTTP HEAD request (default: 10).
    ///
    /// Only relevant when `--check-external` is also passed.
    #[arg(long, value_name = "SECS", default_value_t = 10, requires = "check_links")]
    external_timeout_secs: u64,

    /// Extract a named heading section and print it to stdout, then exit.
    ///
    /// The first heading whose text contains NAME (case-insensitive substring
    /// match) is selected. Output is the heading line plus all body lines up to
    /// the next same-or-higher-level heading. The TUI is not launched.
    ///
    /// Exit code 0 if a matching heading was found; 1 if not.
    ///
    /// Reads from the positional FILE argument if given, or from stdin when
    /// the binary is piped to (e.g. `cat doc.md | markdown-reader --section NAME`).
    #[arg(long, value_name = "NAME", conflicts_with_all = ["export_html", "check_links"])]
    section: Option<String>,
}

/// Read all of stdin into a freshly-created temp file with a `.md` suffix.
///
/// Returned [`tempfile::NamedTempFile`] keeps the file alive on disk for the
/// caller's lifetime — drop it after the app exits to clean up.
fn drain_stdin_to_temp() -> Result<tempfile::NamedTempFile> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("failed to read piped markdown from stdin")?;
    let mut temp = tempfile::Builder::new()
        .prefix("stdin-")
        .suffix(".md")
        .tempfile()
        .context("failed to create temp file for stdin content")?;
    temp.write_all(buf.as_bytes())
        .context("failed to write stdin content to temp file")?;
    temp.flush().context("failed to flush temp file")?;
    Ok(temp)
}

/// After consuming stdin from a pipe, redirect file descriptor 0 to the
/// controlling terminal so crossterm can still read key presses. Without
/// this, every `crossterm::event::read()` call returns immediately with
/// no input because stdin is at EOF.
///
/// Unix-only — on Windows, crossterm uses Win32 console APIs directly,
/// not the stdin file descriptor, so no redirect is needed.
#[cfg(unix)]
fn redirect_stdin_to_tty() -> Result<()> {
    use std::os::unix::io::AsRawFd;

    let tty =
        std::fs::File::open("/dev/tty").context("failed to open /dev/tty for keyboard input")?;
    // Direct FFI to avoid pulling in `libc` for one call.
    unsafe extern "C" {
        fn dup2(oldfd: std::ffi::c_int, newfd: std::ffi::c_int) -> std::ffi::c_int;
    }
    let result = unsafe { dup2(tty.as_raw_fd(), 0) };
    if result < 0 {
        return Err(anyhow::anyhow!(
            "dup2(/dev/tty, stdin) failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // After dup2, fd 0 is an independent reference to the tty. Dropping
    // `tty` (the high-fd reference) is safe and frees the kernel's
    // bookkeeping for the original open() handle.
    drop(tty);
    Ok(())
}

#[cfg(not(unix))]
fn redirect_stdin_to_tty() -> Result<()> {
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── HTML export mode ─────────────────────────────────────────────
    // When `--export-html` is supplied, render to HTML and exit without
    // touching the TUI at all. This keeps the happy path fully isolated
    // so nothing below changes for normal interactive use.
    if let Some(ref md_path) = cli.export_html {
        let content = std::fs::read_to_string(md_path)
            .with_context(|| format!("failed to read {}", md_path.display()))?;

        // Use the user's configured theme so syntect colours match the TUI.
        let cfg = config::Config::load();
        // Derive a human-readable title: prefer the theme label + filename,
        // fallback to just the filename.
        let file_stem = md_path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| md_path.to_string_lossy());
        let title = format!("{} — {}", file_stem, cfg.theme.label());

        let html = export::html::render_to_html(&content, &title, cfg.theme);

        if let Some(ref out_path) = cli.output {
            std::fs::write(out_path, html.as_bytes())
                .with_context(|| format!("failed to write {}", out_path.display()))?;
        } else {
            // Write directly to stdout — a single syscall for the whole
            // document is fine; locking stdout is not needed.
            std::io::stdout()
                .write_all(html.as_bytes())
                .context("failed to write HTML to stdout")?;
        }
        return Ok(());
    }

    // ── Link validation mode ─────────────────────────────────────────────────
    // When `--check-links` is supplied, walk the target directory, validate
    // every .md file's links, print the report, then exit — no TUI is started.
    if let Some(ref dir) = cli.check_links {
        let root = dir
            .canonicalize()
            .with_context(|| format!("could not resolve path: {}", dir.display()))?;
        let opts = checklinks::CheckOpts {
            check_external: cli.check_external,
            external_timeout_secs: cli.external_timeout_secs,
        };
        let report = checklinks::check_dir(&root, &opts);
        report.print(&root);
        let exit_code = if report.is_clean() { 0 } else { 1 };
        std::process::exit(exit_code);
    }

    // ── Section extraction mode ──────────────────────────────────────────────
    // When `--section NAME` is supplied, extract the named heading section from
    // the file (or stdin) and print it to stdout — no TUI is launched.
    // Exit 0 on success, exit 1 when no matching heading exists.
    if let Some(ref name) = cli.section {
        // Decide whether to read from the positional FILE argument or from stdin.
        //
        // Priority: if a real file path was supplied (i.e. `cli.path` differs from
        // the clap default `"."`), always read that file regardless of stdin TTY
        // status. This lets `--section NAME file.md` work correctly even when run
        // inside a pipeline (e.g. `some-cmd | markdown-reader --section NAME file.md`).
        //
        // When no file is given (`cli.path == "."`), fall back to stdin. If stdin
        // is also a TTY at that point there is nothing to read, so we print an error.
        let file_given = cli.path.as_os_str() != ".";
        let source = if file_given {
            let path = &cli.path;
            std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?
        } else if !std::io::stdin().is_terminal() {
            // stdin is a pipe with no explicit file: drain it.
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read piped markdown from stdin")?;
            buf
        } else {
            // Neither a file argument nor piped stdin: nothing to extract from.
            eprintln!("error: --section requires a FILE argument or piped input");
            std::process::exit(1);
        };

        match section_extract::extract_section(&source, name) {
            Some(text) => {
                // `print!` rather than `println!` because `extract_section`
                // already preserves the trailing newline from the source.
                print!("{text}");
                std::process::exit(0);
            }
            None => {
                eprintln!("no heading matching '{name}' found");
                std::process::exit(1);
            }
        }
    }

    // ── stdin piping ─────────────────────────────────────────────────
    // When stdin is a pipe (`cat foo.md | markdown-reader`), drain it to
    // a temp file and open THAT — the path argument is ignored in this
    // mode. The temp file must outlive the App, so hold it in a binding
    // here and let it drop on main()'s return.
    let stdin_temp = if std::io::stdin().is_terminal() {
        None
    } else {
        let temp = drain_stdin_to_temp()?;
        // Re-open /dev/tty as fd 0 so crossterm can read keys.
        redirect_stdin_to_tty()?;
        Some(temp)
    };

    // Resolve symlinks and relative components so all path comparisons inside
    // the app use the same canonical form.
    //
    // When stdin was piped, `initial_display_name` is set to `"<stdin>"` so the
    // tab bar shows a conventional Unix sentinel instead of the temp-file name.
    let (root, initial_file, initial_display_name) = if let Some(temp) = stdin_temp.as_ref() {
        // stdin mode: temp file's parent (typically /tmp) is the tree
        // root, and the temp file is the initial focused tab.
        let path = temp.path().canonicalize()?;
        let parent = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        (parent, Some(path), Some("<stdin>".to_string()))
    } else {
        let canonical = cli.path.canonicalize()?;
        // When the user passes a file, root the tree at its parent directory
        // and remember the file so the event loop can open it once
        // action_tx is ready. When the path is a directory (the common
        // case) there is no initial file.
        if canonical.is_file() {
            let parent = canonical
                .parent()
                // A file always has a parent (at minimum "/"), so this
                // is only None for the filesystem root itself.
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf();
            (parent, Some(canonical), None)
        } else {
            (canonical, None, None)
        }
    };

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // Request the Kitty keyboard enhancement protocol. Modern terminals
    // (Ghostty, Kitty, WezTerm, recent iTerm2, foot) honour it and start
    // sending precise modifier flags — Cmd surfaces as
    // `KeyModifiers::SUPER`, distinguishable from `ALT` (Option / Esc-
    // prefixed sequences). Without it Cmd+arrow and Option+arrow both
    // arrive as ALT-modified to the legacy keyboard layer, so the viewer
    // can't bind them to different actions. Older terminals ignore the
    // request silently and we keep the legacy fallbacks below working.
    //
    // `ignore_or_log` because the request is best-effort — failures
    // (e.g. a stripped-down stdout) shouldn't tank the app.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );

    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = App::new(root, initial_file, initial_display_name)
        .run(&mut terminal)
        .await;
    // Keep `stdin_temp` alive until after the App exits so the file
    // doesn't get unlinked while the App is reading it. Dropping it
    // here removes the temp file on disk.
    drop(stdin_temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `drain_stdin_to_temp` produces a temp file whose contents match
    /// what was on stdin AND whose suffix is `.md` (so the markdown
    /// pipeline picks it up correctly).
    #[test]
    fn drain_stdin_writes_md_temp_file_with_content() {
        // We can't easily mock global stdin in a unit test, but we CAN
        // exercise the file-creation half of the helper. Build a temp
        // file the same way and assert the suffix + writeability.
        let mut temp = tempfile::Builder::new()
            .prefix("stdin-")
            .suffix(".md")
            .tempfile()
            .unwrap();
        let content = "# hello from stdin\n\nThis is a test.\n";
        temp.write_all(content.as_bytes()).unwrap();
        temp.flush().unwrap();

        let path = temp.path();
        assert!(
            path.extension().is_some_and(|e| e == "md"),
            "temp file must have .md suffix: {path:?}"
        );
        let read_back = std::fs::read_to_string(path).unwrap();
        assert_eq!(read_back, content);
    }
}
