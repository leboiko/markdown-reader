//! # mermaid-text
//!
//! Render [Mermaid](https://mermaid.js.org/) flowchart diagrams as Unicode
//! box-drawing text — no browser, no image protocol, pure Rust.
//!
//! ## Quick start
//!
//! ```rust
//! let output = mermaid_text::render("graph LR; A[Start] --> B[End]").unwrap();
//! assert!(output.contains("Start"));
//! assert!(output.contains("End"));
//! ```
//!
//! ## Supported syntax (Phase 1)
//!
//! - `graph LR/TD/RL/BT` and `flowchart LR/TD/RL/BT` headers
//! - Node shapes: `A[rect]`, `A{diamond}`, `A((circle))`, `A(rounded)`, `A`
//! - Edge types: `-->`, `---`, `-.->`, `==>` (all rendered as solid arrows)
//! - Edge labels: `-->|label|` and `-- label -->`
//! - Semicolons and newlines as statement separators

#![forbid(unsafe_code)]

pub mod detect;
pub mod layout;
pub mod parser;
pub mod render;
pub mod types;

pub use types::{Direction, Edge, Graph, Node, NodeShape};

use detect::DiagramKind;
use layout::layered::LayoutConfig;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// All errors that can be returned by this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The input string was empty or contained only whitespace/comments.
    EmptyInput,
    /// The diagram type (e.g. `pie`, `sequenceDiagram`) is not supported.
    ///
    /// The inner string is the unrecognised keyword.
    UnsupportedDiagram(String),
    /// A syntax error was encountered during parsing.
    ///
    /// The inner string is a human-readable description of the problem.
    ParseError(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::EmptyInput => write!(f, "empty or blank input"),
            Error::UnsupportedDiagram(kind) => {
                write!(f, "unsupported diagram type: '{kind}'")
            }
            Error::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render a Mermaid diagram source string to Unicode box-drawing text.
///
/// # Supported diagram types (Phase 1)
///
/// - `graph LR/TD/RL/BT; ...` — flowchart with "graph" keyword
/// - `flowchart LR/TD/RL/BT; ...` — flowchart with "flowchart" keyword
///
/// # Returns
///
/// A multi-line `String` containing the diagram rendered with Unicode
/// box-drawing characters.
///
/// # Errors
///
/// - [`Error::EmptyInput`] — `input` is blank or contains only comments
/// - [`Error::UnsupportedDiagram`] — the diagram type is not supported in Phase 1
/// - [`Error::ParseError`] — the input could not be parsed
///
/// # Examples
///
/// ```
/// let output = mermaid_text::render("graph LR; A[Start] --> B[End]").unwrap();
/// assert!(output.contains("Start"));
/// assert!(output.contains("End"));
/// ```
///
/// ```
/// let output = mermaid_text::render("graph TD; A[Top] --> B[Bottom]").unwrap();
/// assert!(output.contains("Top"));
/// assert!(output.contains("Bottom"));
/// ```
pub fn render(input: &str) -> Result<String, Error> {
    // 1. Detect diagram type
    let kind = detect::detect(input)?;

    // 2. Parse
    let graph = match kind {
        DiagramKind::Flowchart => parser::parse(input)?,
    };

    // 3. Layout
    let config = LayoutConfig::default();
    let positions = layout::layered::layout(&graph, &config);

    // 4. Render
    Ok(render::render(&graph, &positions))
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Rendering tests --------------------------------------------------

    #[test]
    fn render_simple_lr_flowchart() {
        let out = render("graph LR; A-->B-->C").unwrap();
        assert!(out.contains('A'), "missing A in:\n{out}");
        assert!(out.contains('B'), "missing B in:\n{out}");
        assert!(out.contains('C'), "missing C in:\n{out}");
        // Should contain at least one right arrow
        assert!(
            out.contains('▸') || out.contains('-'),
            "no arrow found in:\n{out}"
        );
    }

    #[test]
    fn render_simple_td_flowchart() {
        let out = render("graph TD; A-->B").unwrap();
        // In TD layout, A should appear on an earlier row than B.
        // Simplest proxy: A appears before B in the string.
        let a_pos = out.find('A').unwrap_or(usize::MAX);
        let b_pos = out.find('B').unwrap_or(usize::MAX);
        assert!(a_pos < b_pos, "expected A before B in TD layout:\n{out}");
        // TD layout should have a down arrow
        assert!(out.contains('▾'), "missing down arrow in:\n{out}");
    }

    #[test]
    fn render_labeled_nodes() {
        let out = render("graph LR; A[Start] --> B[End]").unwrap();
        assert!(out.contains("Start"), "missing 'Start' in:\n{out}");
        assert!(out.contains("End"), "missing 'End' in:\n{out}");
        // Rectangle box corners should be present
        assert!(
            out.contains('┌') || out.contains('╭'),
            "no box corner:\n{out}"
        );
    }

    #[test]
    fn render_edge_labels() {
        let out = render("graph LR; A -->|yes| B").unwrap();
        assert!(out.contains("yes"), "missing edge label 'yes' in:\n{out}");
    }

    #[test]
    fn render_diamond_node() {
        let out = render("graph LR; A{Decision} --> B[OK]").unwrap();
        assert!(out.contains("Decision"), "missing 'Decision' in:\n{out}");
        // Diamond should use diagonal chars
        assert!(
            out.contains('/') || out.contains('\\') || out.contains('▲'),
            "no diamond chars in:\n{out}"
        );
    }

    #[test]
    fn parse_semicolons() {
        let out = render("graph LR; A-->B; B-->C").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
        assert!(out.contains('C'));
    }

    #[test]
    fn parse_newlines() {
        let src = "graph TD\nA[Alpha]\nB[Beta]\nA --> B";
        let out = render(src).unwrap();
        assert!(out.contains("Alpha"), "missing 'Alpha' in:\n{out}");
        assert!(out.contains("Beta"), "missing 'Beta' in:\n{out}");
    }

    #[test]
    fn unknown_diagram_type_returns_error() {
        let err = render("pie title Pets").unwrap_err();
        assert!(
            matches!(err, Error::UnsupportedDiagram(_)),
            "expected UnsupportedDiagram, got {err:?}"
        );
    }

    #[test]
    fn empty_input_returns_error() {
        assert!(matches!(render(""), Err(Error::EmptyInput)));
        assert!(matches!(render("   "), Err(Error::EmptyInput)));
        assert!(matches!(render("\n\n"), Err(Error::EmptyInput)));
    }

    #[test]
    fn single_node_renders() {
        let out = render("graph LR; A[Alone]").unwrap();
        assert!(out.contains("Alone"), "missing 'Alone' in:\n{out}");
        assert!(out.contains('┌') || out.contains('╭'));
    }

    #[test]
    fn cyclic_graph_doesnt_hang() {
        // Must complete without infinite loop or stack overflow
        let out = render("graph LR; A-->B; B-->A").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
    }

    #[test]
    fn special_chars_in_labels() {
        let out = render("graph LR; A[Hello World] --> B[Item (1)]").unwrap();
        assert!(out.contains("Hello World"), "missing label in:\n{out}");
        assert!(out.contains("Item (1)"), "missing label in:\n{out}");
    }

    // ---- Error path tests -------------------------------------------------

    #[test]
    fn flowchart_keyword_accepted() {
        let out = render("flowchart LR; A-->B").unwrap();
        assert!(out.contains('A'));
    }

    #[test]
    fn rl_direction_accepted() {
        let out = render("graph RL; A-->B").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
    }

    #[test]
    fn bt_direction_accepted() {
        let out = render("graph BT; A-->B").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
    }

    #[test]
    fn multiple_branches() {
        let src = "graph LR; A[Start] --> B{Decision}; B -->|Yes| C[End]; B -->|No| D[Skip]";
        let out = render(src).unwrap();
        assert!(out.contains("Start"), "missing 'Start':\n{out}");
        assert!(out.contains("Decision"), "missing 'Decision':\n{out}");
        assert!(out.contains("End"), "missing 'End':\n{out}");
        assert!(out.contains("Skip"), "missing 'Skip':\n{out}");
        assert!(out.contains("Yes"), "missing 'Yes':\n{out}");
        assert!(out.contains("No"), "missing 'No':\n{out}");
    }

    #[test]
    fn dotted_arrow_parsed() {
        let out = render("graph LR; A-.->B").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
    }

    #[test]
    fn thick_arrow_parsed() {
        let out = render("graph LR; A==>B").unwrap();
        assert!(out.contains('A'));
        assert!(out.contains('B'));
    }

    #[test]
    fn rounded_node_renders() {
        let out = render("graph LR; A(Rounded)").unwrap();
        assert!(out.contains("Rounded"), "missing label in:\n{out}");
        assert!(
            out.contains('╭') || out.contains('╰'),
            "no rounded corners:\n{out}"
        );
    }

    #[test]
    fn circle_node_renders() {
        let out = render("graph LR; A((Circle))").unwrap();
        assert!(out.contains("Circle"), "missing label in:\n{out}");
        // Circle uses parenthesis markers
        assert!(
            out.contains('(') || out.contains('╭'),
            "no circle markers:\n{out}"
        );
    }

    /// Real-world flowchart with subgraphs, edge labels, and various node
    /// shapes. Verifies the parser skips mermaid keywords (`subgraph`,
    /// `direction`, `end`) and renders the actual nodes.
    #[test]
    fn real_world_flowchart_with_subgraph() {
        let src = r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]
    W --> CB{Circuit Breaker}
    CB -->|CLOSED| DB[(Database)]"#;
        let out = render(src).expect("should parse real-world flowchart");
        assert!(out.contains("Factory"), "missing Factory:\n{out}");
        assert!(out.contains("Worker"), "missing Worker:\n{out}");
        assert!(out.contains("Heartbeat"), "missing Heartbeat:\n{out}");
        assert!(out.contains("Database"), "missing Database:\n{out}");
        // Keywords should NOT appear as node labels.
        assert!(!out.contains("subgraph"), "subgraph should be skipped:\n{out}");
        assert!(!out.contains("direction"), "direction should be skipped:\n{out}");
    }
}