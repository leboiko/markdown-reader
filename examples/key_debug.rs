//! Tiny key-event echo tool. Run with `cargo run --example key_debug`.
//!
//! Prints every key event crossterm receives — useful for diagnosing terminal
//! quirks (does Option send ALT, META, or a literal Esc-prefix? Does Cmd
//! pass through as SUPER, or get swallowed by the terminal entirely?).
//!
//! Press Ctrl+C or Ctrl+D to exit.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, Write};

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    println!("\r\n  key_debug: press any key. Ctrl+C / Ctrl+D quits.\r\n");
    println!("  Format: Code = ...     Modifiers = ...\r");
    println!("  --------------------------------------------------\r");
    io::stdout().flush()?;

    loop {
        if let Event::Key(k) = event::read()? {
            print_key(&k);
            // Quit on Ctrl+C or Ctrl+D so the user can always escape.
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('d'))
            {
                break;
            }
        }
    }

    disable_raw_mode()?;
    println!("\r\n  bye.\r");
    Ok(())
}

fn print_key(k: &KeyEvent) {
    let mods = format_mods(k.modifiers);
    println!("  Code = {:?}    Modifiers = {}\r", k.code, mods);
}

fn format_mods(m: KeyModifiers) -> String {
    if m.is_empty() {
        return "(none)".to_string();
    }
    let mut parts = Vec::new();
    if m.contains(KeyModifiers::SHIFT) {
        parts.push("SHIFT");
    }
    if m.contains(KeyModifiers::CONTROL) {
        parts.push("CONTROL");
    }
    if m.contains(KeyModifiers::ALT) {
        parts.push("ALT");
    }
    if m.contains(KeyModifiers::SUPER) {
        parts.push("SUPER");
    }
    if m.contains(KeyModifiers::HYPER) {
        parts.push("HYPER");
    }
    if m.contains(KeyModifiers::META) {
        parts.push("META");
    }
    parts.join(" + ")
}
