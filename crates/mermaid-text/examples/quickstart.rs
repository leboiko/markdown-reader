//! Quickstart example for `mermaid-text`.
//!
//! Demonstrates the simplest possible usage: render a small flowchart and
//! print it to the terminal.
//!
//! Run with:
//! ```text
//! cargo run -p mermaid-text --example quickstart
//! ```

fn main() {
    // A minimal flowchart in Mermaid's "graph" syntax.
    // `LR` means left-to-right; nodes are separated by semicolons.
    let source = "graph LR; A[Hello] --> B[World]";

    println!("Input Mermaid source:");
    println!("  {source}");
    println!();

    // `render` parses and lays out the diagram, returning a multi-line string.
    // The only reason it can fail is bad syntax or an unsupported diagram type.
    let output = mermaid_text::render(source).expect("valid Mermaid input");

    println!("Rendered output:");
    println!("{output}");
}
