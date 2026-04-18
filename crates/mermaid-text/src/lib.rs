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
//! ## Width-constrained rendering
//!
//! Pass an optional column budget so the renderer tries progressively smaller
//! gap sizes until the output fits:
//!
//! ```rust
//! let output = mermaid_text::render_with_width(
//!     "graph LR; A[Start] --> B[End]",
//!     Some(80),
//! ).unwrap();
//! assert!(output.contains("Start"));
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

pub use types::{Direction, Edge, EdgeEndpoint, EdgeStyle, Graph, Node, NodeShape};

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
// Public entry points
// ---------------------------------------------------------------------------

/// Render a Mermaid diagram source string to Unicode box-drawing text.
///
/// This is a convenience wrapper around [`render_with_width`] that does not
/// apply any column budget — the diagram is rendered at its natural size.
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
    render_with_width(input, None)
}

/// Render a Mermaid diagram source string to Unicode box-drawing text,
/// optionally compacting the output to fit within a column budget.
///
/// When `max_width` is `Some(n)`, the renderer tries progressively smaller
/// gap configurations — from the default down to the minimum — and returns
/// the first result whose longest line is ≤ `n` columns. If no configuration
/// fits, the most compact result is returned anyway (the caller can truncate
/// or scroll as they see fit).
///
/// When `max_width` is `None` the default gap configuration is used and no
/// compaction is attempted.
///
/// # Arguments
///
/// * `input`     — Mermaid source string
/// * `max_width` — optional column budget in terminal cells
///
/// # Errors
///
/// Same as [`render`].
///
/// # Examples
///
/// ```
/// let output = mermaid_text::render_with_width(
///     "graph LR; A[Start] --> B[End]",
///     Some(80),
/// ).unwrap();
/// assert!(output.contains("Start"));
/// ```
pub fn render_with_width(input: &str, max_width: Option<usize>) -> Result<String, Error> {
    // 1. Detect diagram type
    let kind = detect::detect(input)?;

    // 2. Parse (once — reused across compaction attempts)
    let graph = match kind {
        DiagramKind::Flowchart => parser::parse(input)?,
    };

    // 3. Render with default config first.
    let default_cfg = LayoutConfig::default();
    let result = render_with_config(&graph, &default_cfg);

    let Some(budget) = max_width else {
        // No width constraint — return the natural-size rendering.
        return Ok(result);
    };

    if max_line_width(&result) <= budget {
        return Ok(result);
    }

    // 4. Progressive compaction: try smaller gap configurations in order.
    //    Each step reduces both the inter-layer gap and the label padding.
    //    We try four levels; the last one is the most compact.
    const COMPACT_CONFIGS: &[LayoutConfig] = &[
        LayoutConfig {
            layer_gap: 4,
            node_gap: 2,
        },
        LayoutConfig {
            layer_gap: 2,
            node_gap: 1,
        },
        LayoutConfig {
            layer_gap: 1,
            node_gap: 0,
        },
    ];

    // Keep the most compact output in case nothing fits.
    let mut best = render_with_config(&graph, COMPACT_CONFIGS.last().expect("non-empty"));

    for cfg in COMPACT_CONFIGS {
        let candidate = render_with_config(&graph, cfg);
        if max_line_width(&candidate) <= budget {
            return Ok(candidate);
        }
        // Track the last attempt as the fallback.
        best = candidate;
    }

    Ok(best)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Render a pre-parsed `graph` using the given layout configuration.
fn render_with_config(graph: &crate::types::Graph, config: &LayoutConfig) -> String {
    let mut positions = layout::layered::layout(graph, config);

    // When subgraphs are present, offset all positions to ensure there is
    // room for subgraph border padding above/left of the topmost/leftmost
    // subgraph member. Without this, a subgraph whose members start at (0,0)
    // would have a border_row of 0 too (saturating_sub), causing the border
    // to overlap the node boxes.
    if !graph.subgraphs.is_empty() {
        offset_positions_for_subgraphs(graph, &mut positions);
    }

    let sg_bounds = layout::subgraph::compute_subgraph_bounds(graph, &positions);
    render::render(graph, &positions, &sg_bounds)
}

/// Shift all node positions so that the innermost subgraph members have
/// enough space above and to the left for all enclosing subgraph borders.
///
/// Each nesting level needs `SG_BORDER_PAD` cells of breathing room.
/// For a node at depth `d` (inside `d` nested subgraphs), we need at least
/// `SG_BORDER_PAD * (d + 1)` free rows/cols before the node's top-left corner
/// so that every enclosing border can be drawn without `saturating_sub`
/// clipping to 0.
fn offset_positions_for_subgraphs(
    graph: &crate::types::Graph,
    positions: &mut std::collections::HashMap<String, (usize, usize)>,
) {
    use layout::subgraph::SG_BORDER_PAD;

    // Build node → nesting depth (how many subgraphs enclose it).
    let node_sg_map = graph.node_to_subgraph();
    let max_depth = compute_max_nesting_depth(graph);

    // The innermost nodes need `SG_BORDER_PAD * (max_depth + 1)` free space.
    // Nodes outside subgraphs need 0 padding.
    let required_pad = SG_BORDER_PAD * (max_depth + 1);

    // Find the minimum col and row across all nodes that are inside at least
    // one subgraph.
    let mut min_col = usize::MAX;
    let mut min_row = usize::MAX;

    for (node_id, &(col, row)) in positions.iter() {
        if node_sg_map.contains_key(node_id) {
            min_col = min_col.min(col);
            min_row = min_row.min(row);
        }
    }

    if min_col == usize::MAX {
        return; // no subgraph members in positions
    }

    let col_offset = required_pad.saturating_sub(min_col);
    let row_offset = required_pad.saturating_sub(min_row);

    if col_offset == 0 && row_offset == 0 {
        return;
    }

    // Shift every node (not just subgraph members) so the grid remains consistent.
    for (col, row) in positions.values_mut() {
        *col += col_offset;
        *row += row_offset;
    }
}

/// Compute the maximum nesting depth of any subgraph in the graph.
///
/// A top-level subgraph has depth 0; a subgraph inside it has depth 1, etc.
fn compute_max_nesting_depth(graph: &crate::types::Graph) -> usize {
    fn depth_of(graph: &crate::types::Graph, sg: &crate::types::Subgraph, cur: usize) -> usize {
        let mut max = cur;
        for child_id in &sg.subgraph_ids {
            if let Some(child) = graph.find_subgraph(child_id) {
                max = max.max(depth_of(graph, child, cur + 1));
            }
        }
        max
    }

    graph
        .subgraphs
        .iter()
        .map(|sg| depth_of(graph, sg, 0))
        .max()
        .unwrap_or(0)
}

/// Return the maximum display-column width across all lines of `text`.
///
/// Uses [`unicode_width`] so multi-byte characters are counted correctly.
fn max_line_width(text: &str) -> usize {
    text.lines()
        .map(unicode_width::UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
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
        // Diamond renders as a rectangle with ◇ markers at the horizontal
        // centre of the top and bottom edges (termaid convention).
        assert!(out.contains('◇'), "no diamond marker in:\n{out}");
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
        assert!(
            !out.contains("subgraph"),
            "subgraph should be skipped:\n{out}"
        );
        assert!(
            !out.contains("direction"),
            "direction should be skipped:\n{out}"
        );
    }

    /// Verify that multiple edges leaving the same source node in LR direction
    /// each get a distinct exit row, eliminating the ┬┬ clustering artefact.
    #[test]
    fn multiple_edges_from_same_node_spread() {
        let out = render("graph LR; A-->B; A-->C; A-->D").unwrap();
        // Collect the row index of every right-arrow character in the output.
        // With spreading, the three edges should each land on a distinct row.
        let arrow_rows: Vec<usize> = out
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains('▸'))
            .map(|(i, _)| i)
            .collect();
        assert!(
            arrow_rows.len() >= 3,
            "expected at least 3 distinct arrow rows, got {arrow_rows:?}:\n{out}"
        );
        // All rows must be distinct (no two arrows on the same row).
        let unique: std::collections::HashSet<_> = arrow_rows.iter().collect();
        assert_eq!(
            unique.len(),
            arrow_rows.len(),
            "duplicate arrow rows {arrow_rows:?} — edges not spread:\n{out}"
        );
    }

    /// Verify that a long edge label is rendered in full and not truncated.
    #[test]
    fn long_edge_label_not_truncated() {
        let out = render("graph LR; A-->|panics and exits cleanly| B").unwrap();
        assert!(
            out.contains("panics and exits cleanly"),
            "label truncated:\n{out}"
        );
    }

    /// Verify that two labels on edges diverging from the same TD diamond node
    /// do not merge into a single string like `NoYes` or `YesNo`.
    #[test]
    fn diverging_labels_dont_collide() {
        let out = render("graph TD; B{Ok?}; B-->|Yes|C; B-->|No|D").unwrap();
        assert!(out.contains("Yes"), "missing 'Yes' label:\n{out}");
        assert!(out.contains("No"), "missing 'No' label:\n{out}");
        assert!(
            !out.contains("NoYes") && !out.contains("YesNo"),
            "labels collided:\n{out}"
        );
    }

    // ---- Part A: New node shape tests ------------------------------------

    #[test]
    fn stadium_node_renders() {
        let out = render("graph LR; A([Stadium])").unwrap();
        assert!(out.contains("Stadium"), "missing label:\n{out}");
        // Stadium uses rounded corners and ( / ) side markers.
        assert!(
            out.contains('(') || out.contains('╭'),
            "no stadium markers:\n{out}"
        );
    }

    #[test]
    fn subroutine_node_renders() {
        let out = render("graph LR; A[[Subroutine]]").unwrap();
        assert!(out.contains("Subroutine"), "missing label:\n{out}");
        // Subroutine adds inner │ bars next to each side border.
        assert!(out.contains('│'), "no inner vertical bars:\n{out}");
    }

    #[test]
    fn cylinder_node_renders() {
        let out = render("graph LR; A[(Database)]").unwrap();
        assert!(out.contains("Database"), "missing label:\n{out}");
        // Cylinder uses rounded arc chars at top and bottom.
        assert!(
            out.contains('╭') || out.contains('╰'),
            "no cylinder arcs:\n{out}"
        );
    }

    #[test]
    fn hexagon_node_renders() {
        let out = render("graph LR; A{{Hexagon}}").unwrap();
        assert!(out.contains("Hexagon"), "missing label:\n{out}");
        // Hexagon uses < / > markers at the vertical midpoints.
        assert!(out.contains('<') || out.contains('>'), "no hexagon markers:\n{out}");
    }

    #[test]
    fn asymmetric_node_renders() {
        let out = render("graph LR; A>Async]").unwrap();
        assert!(out.contains("Async"), "missing label:\n{out}");
        // Asymmetric uses ⟩ at the right vertical midpoint.
        assert!(out.contains('⟩'), "no asymmetric marker:\n{out}");
    }

    #[test]
    fn parallelogram_node_renders() {
        let out = render("graph LR; A[/Parallel/]").unwrap();
        assert!(out.contains("Parallel"), "missing label:\n{out}");
        // Parallelogram has / markers at top-left and bottom-right corners.
        assert!(out.contains('/'), "no parallelogram slant marker:\n{out}");
    }

    #[test]
    fn trapezoid_node_renders() {
        let out = render("graph LR; A[/Trap\\]").unwrap();
        assert!(out.contains("Trap"), "missing label:\n{out}");
        // Trapezoid has / at top-left and \ at top-right corners.
        assert!(out.contains('/'), "no trapezoid slant marker:\n{out}");
    }

    #[test]
    fn double_circle_node_renders() {
        let out = render("graph LR; A(((DblCircle)))").unwrap();
        assert!(out.contains("DblCircle"), "missing label:\n{out}");
        // Double circle has two concentric rounded borders.
        let corner_count = out.chars().filter(|&c| c == '╭').count();
        assert!(
            corner_count >= 2,
            "expected ≥2 rounded corners for double circle, got {corner_count}:\n{out}"
        );
    }

    // ---- Part B: Edge style tests ----------------------------------------

    #[test]
    fn dotted_edge_renders_with_dotted_glyph() {
        let out = render("graph LR; A-.->B").unwrap();
        // Dotted horizontal should contain ┄ or dotted vertical ┆.
        assert!(
            out.contains('┄') || out.contains('┆'),
            "no dotted glyph in:\n{out}"
        );
    }

    #[test]
    fn thick_edge_renders_with_thick_glyph() {
        let out = render("graph LR; A==>B").unwrap();
        assert!(
            out.contains('━') || out.contains('┃'),
            "no thick glyph in:\n{out}"
        );
    }

    #[test]
    fn bidirectional_edge_has_two_arrows() {
        let out = render("graph LR; A<-->B").unwrap();
        // Should contain both ◂ (pointing back to A) and ▸ (pointing to B).
        assert!(
            out.contains('◂') && out.contains('▸'),
            "missing bidirectional arrows in:\n{out}"
        );
    }

    #[test]
    fn plain_line_edge_has_no_arrow() {
        let out = render("graph LR; A---B").unwrap();
        // No arrow tip characters.
        assert!(
            !out.contains('▸') && !out.contains('◂'),
            "unexpected arrow in plain line:\n{out}"
        );
    }

    #[test]
    fn circle_endpoint_renders_circle_glyph() {
        let out = render("graph LR; A--oB").unwrap();
        assert!(out.contains('○'), "no circle endpoint glyph in:\n{out}");
    }

    #[test]
    fn cross_endpoint_renders_cross_glyph() {
        let out = render("graph LR; A--xB").unwrap();
        assert!(out.contains('×'), "no cross endpoint glyph in:\n{out}");
    }

    // ---- Subgraph tests ---------------------------------------------------

    /// A single subgraph should render with a rounded border and a label at
    /// the top, enclosing all member nodes.
    #[test]
    fn subgraph_renders_with_border_and_label() {
        let src = r#"graph LR
    subgraph Supervisor
        F[Factory] --> W[Worker]
    end"#;
        let out = render(src).unwrap();
        assert!(out.contains("Supervisor"), "missing label:\n{out}");
        assert!(out.contains("Factory"), "missing Factory:\n{out}");
        assert!(out.contains("Worker"), "missing Worker:\n{out}");
        // Subgraph uses rounded corners to distinguish from node boxes.
        assert!(
            out.contains('╭') || out.contains('╰'),
            "missing rounded subgraph corner:\n{out}"
        );
        // The subgraph border should appear as a vertical side bar on the left.
        assert!(out.contains('│'), "missing vertical border:\n{out}");
    }

    /// Two nested subgraphs should both show their labels and the inner border
    /// should be visually contained within the outer one.
    #[test]
    fn nested_subgraphs_render() {
        let src = r#"graph TD
    subgraph Outer
        subgraph Inner
            A[A]
        end
        B[B]
    end"#;
        let out = render(src).unwrap();
        assert!(out.contains("Outer"), "missing Outer label:\n{out}");
        assert!(out.contains("Inner"), "missing Inner label:\n{out}");
        assert!(out.contains('A'), "missing A:\n{out}");
        assert!(out.contains('B'), "missing B:\n{out}");
        // Two levels of rounded corners should appear.
        let corner_count = out.chars().filter(|&c| c == '╭').count();
        assert!(
            corner_count >= 2,
            "expected at least 2 top-left rounded corners (one per subgraph), got {corner_count}:\n{out}"
        );
    }

    /// An edge that crosses a subgraph boundary should render without panicking
    /// and the external node should appear outside the subgraph border.
    #[test]
    fn edge_crossing_subgraph_boundary_renders() {
        let src = r#"graph LR
    subgraph S
        F[Factory] --> W[Worker]
    end
    W --> HB[Heartbeat]"#;
        let out = render(src).unwrap();
        // Heartbeat should be outside the S rectangle; edge from W to HB
        // should exist without the whole thing hanging or panicking.
        assert!(out.contains("Heartbeat"), "missing Heartbeat:\n{out}");
        assert!(out.contains("Factory"), "missing Factory:\n{out}");
        assert!(out.contains("Worker"), "missing Worker:\n{out}");
        // The subgraph border should be present.
        assert!(out.contains('╭'), "missing subgraph border:\n{out}");
    }

    /// `real_world_flowchart_with_subgraph` now exercises the full subgraph
    /// pipeline — nodes inside the Supervisor subgraph should still render,
    /// and the "subgraph"/"direction"/"end" keywords must NOT appear as labels.
    /// (This test was present before and still passes unchanged.)
    #[test]
    fn subgraph_keywords_not_leaked_as_labels() {
        let src = r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat| HB[Heartbeat]"#;
        let out = render(src).expect("should render");
        assert!(out.contains("Factory"), "missing Factory:\n{out}");
        assert!(out.contains("Worker"), "missing Worker:\n{out}");
        assert!(out.contains("Heartbeat"), "missing Heartbeat:\n{out}");
        // The subgraph label "Supervisor" appears in the border, but the
        // bare keyword "subgraph" must not appear as a standalone label.
        assert!(
            !out.contains("subgraph"),
            "bare 'subgraph' keyword leaked into output:\n{out}"
        );
        assert!(
            !out.contains("direction"),
            "bare 'direction' keyword leaked into output:\n{out}"
        );
    }
}
