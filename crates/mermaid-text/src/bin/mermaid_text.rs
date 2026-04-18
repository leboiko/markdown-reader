//! CLI for `mermaid-text`.
//!
//! Reads Mermaid source from stdin or a file path argument and prints the
//! rendered Unicode box-drawing diagram to stdout.
//!
//! # Usage
//!
//! ```text
//! # From a file:
//! mermaid-text diagram.mmd
//!
//! # From stdin:
//! echo "graph LR; A-->B-->C" | mermaid-text
//!
//! # With a column budget:
//! mermaid-text --width 80 diagram.mmd
//! ```

use std::io::Read;
use std::process;

fn main() {
    let mut args = std::env::args().skip(1).peekable();

    // Parse optional --width N flag.
    let mut max_width: Option<usize> = None;
    let mut path: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" | "-w" => {
                let n = args
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("error: --width requires a positive integer argument");
                        process::exit(2);
                    });
                max_width = Some(n);
            }
            "--help" | "-h" => {
                println!("Usage: mermaid-text [--width N] [FILE]");
                println!();
                println!("Render a Mermaid graph/flowchart diagram as Unicode box-drawing text.");
                println!();
                println!("Arguments:");
                println!("  FILE        Path to a .mmd file (reads stdin if omitted)");
                println!();
                println!("Options:");
                println!("  --width N   Compact output to fit within N terminal columns");
                println!("  --help      Print this help message");
                process::exit(0);
            }
            other if !other.starts_with('-') => {
                path = Some(other.to_string());
            }
            other => {
                eprintln!("error: unknown flag '{other}'");
                process::exit(2);
            }
        }
    }

    // Read Mermaid source.
    let source = if let Some(ref file_path) = path {
        std::fs::read_to_string(file_path).unwrap_or_else(|e| {
            eprintln!("error: cannot read '{}': {e}", file_path);
            process::exit(1);
        })
    } else {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
            eprintln!("error: failed to read stdin: {e}");
            process::exit(1);
        });
        buf
    };

    match mermaid_text::render_with_width(&source, max_width) {
        Ok(output) => print!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}
