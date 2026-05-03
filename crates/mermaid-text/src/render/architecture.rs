//! Renderer for [`Architecture`] diagrams.
//!
//! Translates an `architecture-beta` data model into a synthetic flowchart
//! [`Graph`] and delegates to the existing Sugiyama layout + A\* router
//! pipeline — the same engine used by all flowchart diagrams.
//!
//! ## Translation (Path A)
//!
//! | Architecture model | Flowchart model       |
//! |--------------------|-----------------------|
//! | `ArchGroup`        | `Subgraph`            |
//! | `ArchService`      | `Node` (Rectangle)    |
//! | `ArchEdge`         | `Edge`                |
//!
//! Port specifiers (`L`/`R`/`T`/`B` on [`ArchEdge`]) are stored but ignored
//! in this pass — spatial port-aware attachment is deferred to Path B.
//! Junction nodes (`junction(id)`) were silently skipped at parse time and
//! remain absent from the translated graph.
//!
//! ## Path B (deferred)
//!
//! True port-aware routing: constrain each edge to exit/enter the declared
//! face of its service box. Requires custom attach-point logic in
//! `layout/router.rs`. See `ROADMAP.md` for details.

use crate::{
    architecture::{ArchEdge, Architecture},
    layout::{
        self,
        layered::{LayoutConfig, LayoutResult},
        subgraph::compute_subgraph_bounds,
    },
    render as flowchart_render,
    types::{Direction, Edge, EdgeEndpoint, EdgeStyle, Graph, Node, NodeShape, Subgraph},
};

/// Translate an [`Architecture`] diagram into a flowchart [`Graph`] and
/// render it through the Sugiyama layout + A\* router pipeline.
///
/// Port specifiers on edges are intentionally ignored (Path B work).
pub fn render(diag: &Architecture, max_width: Option<usize>) -> String {
    if diag.services.is_empty() && diag.groups.is_empty() {
        return String::new();
    }

    let graph = architecture_to_flowchart_graph(diag);
    render_flowchart_graph(&graph, max_width)
}

/// Convert an [`Architecture`] into a flowchart [`Graph`].
///
/// Direction is `TopToBottom` so groups with multiple services render with a
/// natural vertical arrangement of layers rather than a single wide row.
pub fn architecture_to_flowchart_graph(diag: &Architecture) -> Graph {
    let mut graph = Graph::new(Direction::TopToBottom);

    // Groups become subgraph containers; services within them become direct
    // node_ids members of the matching subgraph.
    for group in &diag.groups {
        let label = group
            .label
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or(&group.id)
            .to_string();
        let mut sg = Subgraph::new(&group.id, label);

        for svc in diag.services_in_group(&group.id) {
            let node_label = svc.display_label().to_string();
            graph
                .nodes
                .push(Node::new(&svc.id, node_label, NodeShape::Rectangle));
            sg.node_ids.push(svc.id.clone());
        }

        graph.subgraphs.push(sg);
    }

    // Top-level services (not in any group) become ungrouped nodes.
    for svc in diag.top_level_services() {
        let node_label = svc.display_label().to_string();
        graph
            .nodes
            .push(Node::new(&svc.id, node_label, NodeShape::Rectangle));
    }

    // Edges: undirected (`--`) map to None→None, directed (`-->`) to None→Arrow.
    // Port specifiers are ignored here (Path B).
    for edge in &diag.edges {
        graph.edges.push(arch_edge_to_flowchart_edge(edge));
    }

    graph
}

/// Map a single [`ArchEdge`] to a flowchart [`Edge`].
///
/// Architecture `--` edges have no visual directionality — both endpoints are
/// `None`. The `-->` form is not yet produced by the architecture parser (all
/// parsed edges are undirected), but if it ever is, the label is forwarded.
fn arch_edge_to_flowchart_edge(edge: &ArchEdge) -> Edge {
    Edge::new_styled(
        &edge.source,
        &edge.target,
        edge.label.clone(),
        EdgeStyle::Solid,
        EdgeEndpoint::None,
        EdgeEndpoint::None,
    )
}

/// Run the Sugiyama layout + A\* router + Unicode renderer on a pre-built
/// flowchart [`Graph`], honouring an optional column budget.
///
/// Architecture-beta edges are typically unlabeled (`--` or `-->`), so the
/// default `layer_gap = 6` used by flowchart diagrams (reserved for edge
/// labels) is wasteful. We start with a tighter config (`layer_gap = 2,
/// node_gap = 1`) and fall back to progressively more compact configs only
/// when the output would exceed `max_width`.
fn render_flowchart_graph(graph: &Graph, max_width: Option<usize>) -> String {
    // Tighter default for architecture-beta: no label clearance needed.
    // The Sugiyama backend has a hardcoded 3-cell baseline gap; layer_gap only
    // adds extra beyond that. Setting layer_gap=2 (below the baseline) means
    // no extra spacing is added — we get the minimum 3-cell gap rather than the
    // default 6 cells. node_gap=1 reduces horizontal sibling spacing slightly.
    let default_cfg = LayoutConfig::with_gaps(2, 1);
    let result = layout_and_render(graph, &default_cfg);

    let Some(budget) = max_width else {
        return result;
    };

    if max_line_width(&result) <= budget {
        return result;
    }

    const COMPACT_CONFIGS: &[LayoutConfig] = &[
        LayoutConfig::with_gaps(1, 1),
        LayoutConfig::with_gaps(1, 0),
    ];

    let mut best = layout_and_render(graph, COMPACT_CONFIGS.last().expect("non-empty"));
    for cfg in COMPACT_CONFIGS {
        let candidate = layout_and_render(graph, cfg);
        if max_line_width(&candidate) <= budget {
            return candidate;
        }
        best = candidate;
    }
    best
}

/// Run the full layout + render pipeline for one configuration.
fn layout_and_render(graph: &Graph, config: &LayoutConfig) -> String {
    let LayoutResult { mut positions, .. } = layout::sugiyama_layout(graph, config);

    if !graph.subgraphs.is_empty() {
        let (col_off, row_off) = subgraph_position_offset(graph, &positions);
        if col_off != 0 || row_off != 0 {
            for (col, row) in positions.values_mut() {
                *col += col_off;
                *row += row_off;
            }
        }
    }

    let sg_bounds = compute_subgraph_bounds(graph, &positions);
    flowchart_render::render(graph, &positions, &sg_bounds)
}

/// Replicate the subgraph position-offset logic from `lib.rs` so that
/// subgraph borders have enough headroom at the top and left.
fn subgraph_position_offset(
    graph: &Graph,
    positions: &std::collections::HashMap<String, (usize, usize)>,
) -> (usize, usize) {
    use layout::subgraph::SG_BORDER_PAD;

    let node_sg_map = graph.node_to_subgraph();
    let max_depth = graph
        .subgraphs
        .iter()
        .map(|sg| subgraph_depth(graph, sg, 0))
        .max()
        .unwrap_or(0);
    let required_pad = SG_BORDER_PAD * (max_depth + 1);

    let mut min_col = usize::MAX;
    let mut min_row = usize::MAX;
    for (node_id, &(col, row)) in positions.iter() {
        if node_sg_map.contains_key(node_id) {
            min_col = min_col.min(col);
            min_row = min_row.min(row);
        }
    }
    if min_col == usize::MAX {
        return (0, 0);
    }
    (
        required_pad.saturating_sub(min_col),
        required_pad.saturating_sub(min_row),
    )
}

fn subgraph_depth(graph: &Graph, sg: &Subgraph, cur: usize) -> usize {
    let mut max = cur;
    for child_id in &sg.subgraph_ids {
        if let Some(child) = graph.find_subgraph(child_id) {
            max = max.max(subgraph_depth(graph, child, cur + 1));
        }
    }
    max
}

fn max_line_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::architecture::parse;

    fn parsed(src: &str) -> Architecture {
        parse(src).expect("parse must succeed")
    }

    #[test]
    fn renders_group_with_services() {
        let src = "architecture-beta
    group api(cloud)[API]
    service db(database)[Database] in api
    service server(server)[Server] in api";
        let arch = parsed(src);
        let out = render(&arch, None);

        // Group label must appear in the subgraph border.
        assert!(out.contains("API"), "group label 'API' missing:\n{out}");
        // Service labels must appear.
        assert!(
            out.contains("Database"),
            "service 'Database' missing:\n{out}"
        );
        assert!(out.contains("Server"), "service 'Server' missing:\n{out}");
        // Box-drawing characters must be present.
        assert!(
            out.contains('\u{250C}'),
            "top-left corner ┌ missing:\n{out}"
        );
        assert!(
            out.contains('\u{2518}'),
            "bottom-right corner ┘ missing:\n{out}"
        );
        assert!(out.contains('\u{2502}'), "vertical bar │ missing:\n{out}");
    }

    #[test]
    fn renders_standalone_top_level_services() {
        let src = "architecture-beta\n    service ext(internet)[External]";
        let arch = parsed(src);
        let out = render(&arch, None);

        assert!(
            out.contains("External"),
            "top-level service label missing:\n{out}"
        );
        assert!(out.contains('\u{250C}'), "top-left corner missing:\n{out}");
    }

    #[test]
    fn empty_diagram_renders_without_panic() {
        let arch = Architecture::default();
        let out = render(&arch, None);
        // An empty diagram has nothing to render — output may be empty.
        assert!(!out.contains('\u{250C}'), "no box for empty diagram");
    }

    /// Path A regression guard: edges must be spatially routed, not summarised
    /// as a "Connections:" text block below the boxes.
    ///
    /// Given one group, two services, and one edge, the output must contain
    /// a spatial edge connector character (─) somewhere between the service
    /// boxes, and must NOT contain a "Connections:" line (that is the old
    /// Phase-1 text-summary format this upgrade replaces).
    #[test]
    fn edges_are_spatially_routed_not_text_summary() {
        let src = "architecture-beta
    group cluster(cloud)[Cluster]
    service svc_a(server)[Alpha] in cluster
    service svc_b(database)[Beta] in cluster
    svc_a -- svc_b";
        let arch = parsed(src);
        let out = render(&arch, None);

        // Service labels must still appear.
        assert!(out.contains("Alpha"), "service Alpha missing:\n{out}");
        assert!(out.contains("Beta"), "service Beta missing:\n{out}");

        // The spatial Sugiyama router draws edges with ─ / │ / ▸ etc.
        // At minimum the horizontal dash must appear (it connects node boxes).
        let has_spatial_edge = out.contains('\u{2500}') // ─
            || out.contains('\u{2502}') // │ (vertical routing)
            || out.contains("▸")        // arrow tip
            || out.contains('>'); // ASCII fallback
        assert!(has_spatial_edge, "no spatial edge connector found:\n{out}");

        // The old Phase-1 text summary must NOT appear.
        assert!(
            !out.contains("Connections:"),
            "old Phase-1 Connections: summary must not appear after Path A upgrade:\n{out}"
        );

        // Stronger property: the edge must produce visible routing CHARACTERS
        // OUTSIDE the service-box rectangles. The previous assertion (any `─`)
        // is satisfied by subgraph borders and box tops/bottoms even when no
        // edges render — we need to count connector chars in the area BETWEEN
        // the two services.
        //
        // Strategy: render the same diagram WITHOUT the edge and compare
        // the count of vertical/horizontal box-drawing chars. A real edge
        // adds at least one new connector glyph.
        let no_edge_src = "architecture-beta
    group cluster(cloud)[Cluster]
    service svc_a(server)[Alpha] in cluster
    service svc_b(database)[Beta] in cluster";
        let no_edge_out = render(&parsed(no_edge_src), None);
        let connector_count = |s: &str| -> usize {
            s.chars()
                .filter(|c| {
                    matches!(
                        *c,
                        '─' | '│'
                            | '┌'
                            | '┐'
                            | '└'
                            | '┘'
                            | '├'
                            | '┤'
                            | '┬'
                            | '┴'
                            | '┼'
                            | '▸'
                            | '▴'
                            | '▾'
                            | '◂'
                    )
                })
                .count()
        };
        let with_edge = connector_count(&out);
        let without_edge = connector_count(&no_edge_out);
        assert!(
            with_edge > without_edge,
            "edge added 0 visible routing connectors (with_edge={with_edge}, without_edge={without_edge}). Edge wasn't actually drawn — translator likely lost the edge or router skipped it.\nWITH edge:\n{out}\nWITHOUT edge:\n{no_edge_out}"
        );
    }

    /// Translator unit test: the graph built from a small architecture diagram
    /// has the expected node count, subgraph count, and edge count.
    #[test]
    fn architecture_to_flowchart_graph_mapping() {
        let src = "architecture-beta
    group g1(cloud)[Group1]
    service s1(server)[Svc1] in g1
    service s2(database)[Svc2] in g1
    service s3(internet)[Standalone]
    s1 -- s3";
        let arch = parsed(src);
        let graph = architecture_to_flowchart_graph(&arch);

        assert_eq!(graph.nodes.len(), 3, "s1 + s2 + s3");
        assert_eq!(graph.subgraphs.len(), 1, "one group");
        assert_eq!(graph.subgraphs[0].node_ids, vec!["s1", "s2"]);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "s1");
        assert_eq!(graph.edges[0].to, "s3");
        // Undirected architecture edge → both endpoints None.
        assert_eq!(graph.edges[0].start, crate::types::EdgeEndpoint::None);
        assert_eq!(graph.edges[0].end, crate::types::EdgeEndpoint::None);
    }
}
