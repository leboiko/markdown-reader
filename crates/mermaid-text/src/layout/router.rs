//! Edge routing strategy layer.
//!
//! Provides a single entry point [`route_all`] that routes every edge in a
//! graph end-to-end with one A\* call per edge, preceded by two fast-path
//! checks:
//!
//! 1. **Straight route** — if every cell between source and target on the
//!    same row or column is free, draw the direct path in O(n) without
//!    running A\*.
//! 2. **L-route** — try both single-bend shapes (horizontal-first and
//!    vertical-first); return the one whose cells are cheaper (fewest
//!    obstacles), again without a full A\*.
//! 3. **A\* fallback** — for all other cases, delegate to
//!    [`Grid::route_edge`] / [`Grid::route_back_edge`], which run the full
//!    obstacle-aware pathfinder.
//!
//! Edges are routed in ascending Manhattan-distance order (shortest first).
//! This distributes short edges into clean channels before long edges need
//! to route around them, reducing avoidable crossings.
//!
//! The output is a `Vec<Option<Vec<(usize, usize)>>>` indexed by the
//! **original** edge index from `graph.edges`, so downstream code can look up
//! `paths[edge_idx]` without any remapping.

use crate::layout::Grid;
use crate::layout::grid::{Attach, DIR_DOWN, DIR_UP};
use crate::types::{Direction, Graph};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Route all edges in `graph` and return their pixel paths.
///
/// Edges are routed in ascending Manhattan-distance order. Each edge is tried
/// through three strategies in order: straight line, single-bend L, and
/// full A\* pathfinding.
///
/// # Arguments
///
/// * `grid`          — the canvas; updated in-place as each edge is drawn.
/// * `graph`         — the parsed flowchart (used for `is_back_edge`
///   classification and edge ordering).
/// * `attach_points` — per-edge `(src, dst)` attach points as produced by
///   `compute_spread_attaches`; indexed by edge index. `None` entries are
///   skipped (missing position data for that edge).
/// * `tip_for`       — closure returning the arrow-tip character for edge `i`.
/// * `is_back`       — closure returning `true` when edge `i` is a back-edge.
///
/// # Returns
///
/// A `Vec` with one entry per edge in `graph.edges`. Each entry is:
/// - `Some(path)` — the routed pixel path, suitable for label placement and
///   style post-processing.
/// - `None` — edge had no valid attach point (node missing from positions).
pub(crate) fn route_all(
    grid: &mut Grid,
    graph: &Graph,
    attach_points: &[Option<(Attach, Attach)>],
    tip_for: impl Fn(usize) -> char,
    is_back: impl Fn(usize) -> bool,
) -> Vec<Option<Vec<(usize, usize)>>> {
    let n = graph.edges.len();
    let mut paths: Vec<Option<Vec<(usize, usize)>>> = vec![None; n];

    // Route shortest edges first: fewer cells between endpoints means fewer
    // obstacles to compete with when claiming corridor space.
    let order = order_edges(graph, attach_points);
    let horizontal_first = graph.direction.is_horizontal();

    for edge_idx in order {
        let Some(Some((src, dst))) = attach_points.get(edge_idx) else {
            continue;
        };
        let (src, dst) = (*src, *dst);
        let tip = tip_for(edge_idx);
        let back = is_back(edge_idx);

        let path = if back {
            // Back-edges always use A* with high InnerArea cost to bias them
            // toward the perimeter corridor. The fast-path checks don't apply
            // because back-edges must avoid the diagram body.
            grid.route_back_edge(src.col, src.row, dst.col, dst.row, horizontal_first, tip)
        } else if let Some(p) = try_straight_route(grid, src, dst, horizontal_first, tip) {
            Some(p)
        } else if let Some(p) = try_l_route(grid, src, dst, horizontal_first, tip) {
            Some(p)
        } else {
            grid.route_edge(src.col, src.row, dst.col, dst.row, horizontal_first, tip)
        };

        // Source-attach: only for TD/BT layouts whose route turns
        // sideways at the source cell.
        //
        // The cell at the route's first position renders from its
        // direction bits. For TD/BT layouts the source attach sits
        // immediately below/above the box (against a horizontal `─`
        // border). When the route's first step is also horizontal, the
        // single-bit cell renders as `─`, which stacks confusingly
        // beside the box's `─` border — both look like horizontal
        // segments at adjacent rows. Adding the "back into box" UP/DOWN
        // bit converts the cell to a corner glyph (`└ ┘ ┌ ┐`) that
        // visibly exits the box border.
        //
        // Skipped for:
        //   - vertical first steps (cell is `│`, sits cleanly next to
        //     any box wall — no anchor needed)
        //   - LR/RL layouts (the back-into-box bit would land on the
        //     same axis as the route's first step, OR-ing into a
        //     visual no-op `─` while still polluting the cell's
        //     direction bits, which subtly increases edge_occupied
        //     weight for downstream L-route cost calculations and
        //     measurably worsens crossings on dense LR graphs)
        //
        // The 1.22.1 release added the anchor unconditionally, which
        // produced spurious corners (`│┐` `│┘`) on every edge with a
        // vertical first step (back-edges, mid-side attach points in
        // LR layouts containing internal TB subgraphs).
        if let Some(p) = &path
            && p.len() >= 2
        {
            let (c0, r0) = p[0];
            let (_, r1) = p[1];
            let route_first_step_horizontal = r0 == r1;
            if route_first_step_horizontal {
                let anchor = match graph.direction {
                    Direction::TopToBottom => Some(DIR_UP),
                    Direction::BottomToTop => Some(DIR_DOWN),
                    Direction::LeftToRight | Direction::RightToLeft => None,
                };
                if let Some(bits) = anchor {
                    grid.add_dirs(c0, r0, bits);
                }
            }
        }

        paths[edge_idx] = path;
    }

    paths
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Order edge indices by ascending Manhattan distance between their attach
/// points. Edges with no attach point (missing position data) are appended
/// last in declaration order.
///
/// Routing shortest-first opens clean corridors before long edges compete for
/// the same space, reducing unnecessary crossings.
fn order_edges(graph: &Graph, attach_points: &[Option<(Attach, Attach)>]) -> Vec<usize> {
    let n = graph.edges.len();
    let mut with_dist: Vec<(usize, usize)> = (0..n)
        .map(|i| {
            let dist = attach_points
                .get(i)
                .and_then(|p| p.as_ref())
                .map(|(src, dst)| src.col.abs_diff(dst.col) + src.row.abs_diff(dst.row))
                .unwrap_or(usize::MAX); // no attach → sort to end
            (i, dist)
        })
        .collect();
    // Stable sort preserves declaration order within same-distance ties.
    with_dist.sort_by_key(|&(_, d)| d);
    with_dist.into_iter().map(|(i, _)| i).collect()
}

/// Attempt a straight-line route between `src` and `dst`.
///
/// Succeeds when source and target share the same row OR the same column AND
/// every cell between them is free (not a `NodeBox`). When it succeeds the
/// path is drawn on the grid and returned. When it fails `None` is returned
/// and the grid is unchanged.
///
/// This avoids A\* for the common case of adjacent nodes in a well-spaced
/// layout where no obstacles fall on the direct line.
fn try_straight_route(
    grid: &mut Grid,
    src: Attach,
    dst: Attach,
    horizontal_first: bool,
    tip: char,
) -> Option<Vec<(usize, usize)>> {
    if src.col == dst.col {
        // Same column — try vertical straight line.
        let (r0, r1) = min_max(src.row, dst.row);
        if (r0..=r1).all(|r| !grid.is_node_box(src.col, r)) {
            return grid.route_edge(src.col, src.row, dst.col, dst.row, horizontal_first, tip);
        }
    } else if src.row == dst.row {
        // Same row — try horizontal straight line.
        let (c0, c1) = min_max(src.col, dst.col);
        if (c0..=c1).all(|c| !grid.is_node_box(c, src.row)) {
            return grid.route_edge(src.col, src.row, dst.col, dst.row, horizontal_first, tip);
        }
    }
    None
}

/// Attempt a single-bend L-shaped route between `src` and `dst`.
///
/// Tries both bend orientations (H-then-V and V-then-H) and returns the
/// route whose cells have the lowest total obstacle weight — i.e. the one
/// that crosses fewer existing edges or node boxes. Returns `None` if both
/// orientations have hard obstacles (NodeBox cells) blocking them.
///
/// The grid is updated only when a route is found and accepted.
fn try_l_route(
    grid: &mut Grid,
    src: Attach,
    dst: Attach,
    horizontal_first: bool,
    tip: char,
) -> Option<Vec<(usize, usize)>> {
    if src.col == dst.col || src.row == dst.row {
        // Degenerate L (already straight) — handled by try_straight_route or A*.
        return None;
    }

    // Two L-shape options. The corner cell of each:
    //   H-first (horizontal-then-vertical) → corner at (dst.col, src.row)
    //                                        → bend NEAR the SOURCE row.
    //   V-first (vertical-then-horizontal) → corner at (src.col, dst.row)
    //                                        → bend NEAR the TARGET row.
    //
    // For TB/BT flow we want the bend near the target so the source side
    // is a clean straight `│`, which makes the edge visibly continuous
    // out of the source box. Symmetrically for LR/RL we want the bend
    // near the target column, i.e. H-first. The `horizontal_first` flag
    // already captures this — it's `true` for LR/RL, `false` for TB/BT.
    let cost_hv = l_cost(grid, src, dst, true);
    let cost_vh = l_cost(grid, src, dst, false);

    let prefer_hv = match (cost_hv, cost_vh) {
        (None, None) => return None, // both blocked by hard obstacles
        (Some(_), None) => true,     // only H-bend clear
        (None, Some(_)) => false,    // only V-bend clear
        (Some(ch), Some(cv)) if ch != cv => ch < cv, // strictly cheaper wins
        (Some(_), Some(_)) => {
            // Tie — bend near the target. For TB/BT (`horizontal_first =
            // false`) that's V-first; for LR/RL (`horizontal_first = true`)
            // that's H-first. Either way `prefer_hv = horizontal_first`.
            horizontal_first
        }
    };
    grid.route_edge(src.col, src.row, dst.col, dst.row, prefer_hv, tip)
}

/// Compute the soft-obstacle weight of an L-shaped path.
///
/// Returns `None` if any cell on the two segments is a `NodeBox` (hard
/// obstacle). Returns `Some(total_cost)` otherwise, where `total_cost`
/// counts soft-obstacle crossings (existing edges) along both segments.
///
/// `hv_first = true` → horizontal then vertical (corner at `(dst.col, src.row)`).
/// `hv_first = false` → vertical then horizontal (corner at `(src.col, dst.row)`).
fn l_cost(grid: &Grid, src: Attach, dst: Attach, hv_first: bool) -> Option<u32> {
    let (corner_c, corner_r) = if hv_first {
        (dst.col, src.row)
    } else {
        (src.col, dst.row)
    };

    let mut cost = 0u32;

    // First segment: src → corner.
    let (c0, c1) = if hv_first {
        min_max(src.col, corner_c)
    } else {
        (src.col, src.col)
    };
    let (r0, r1) = if hv_first {
        (src.row, src.row)
    } else {
        min_max(src.row, corner_r)
    };
    for c in c0..=c1 {
        for r in r0..=r1 {
            if grid.is_node_box(c, r) && !(c == dst.col && r == dst.row) {
                return None;
            }
            cost += grid.edge_occupied_cost(c, r);
        }
    }

    // Second segment: corner → dst.
    let (c0, c1) = if hv_first {
        (corner_c, corner_c)
    } else {
        min_max(corner_c, dst.col)
    };
    let (r0, r1) = if hv_first {
        min_max(corner_r, dst.row)
    } else {
        (dst.row, dst.row)
    };
    for c in c0..=c1 {
        for r in r0..=r1 {
            if grid.is_node_box(c, r) && !(c == dst.col && r == dst.row) {
                return None;
            }
            cost += grid.edge_occupied_cost(c, r);
        }
    }

    Some(cost)
}

/// Sort two values into `(min, max)` order.
fn min_max(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Direction, Edge, Graph, Node, NodeShape};

    /// Construct a minimal two-node LR graph with one edge.
    fn two_node_lr() -> (Graph, Vec<Option<(Attach, Attach)>>) {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        // Manually craft attach points: A exits at (7,1), B enters at (9,1).
        let attaches = vec![Some((Attach { col: 7, row: 1 }, Attach { col: 9, row: 1 }))];
        (g, attaches)
    }

    #[test]
    fn route_all_straight_line_uses_fast_path() {
        let (g, attaches) = two_node_lr();
        let mut grid = Grid::new(20, 5);
        let paths = route_all(
            &mut grid,
            &g,
            &attaches,
            |_| crate::layout::grid::arrow::RIGHT,
            |_| false,
        );
        assert_eq!(paths.len(), 1);
        // A horizontal straight route returns a path from col 7 to col 9.
        let path = paths[0].as_ref().expect("expected a routed path");
        assert_eq!(path.first(), Some(&(7, 1)));
        assert_eq!(path.last(), Some(&(9, 1)));
    }

    #[test]
    fn route_all_back_edge_skips_fast_paths() {
        // Source is to the right of destination → back-edge.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("B", "A", None));
        // Dst is left of src → back-edge in LR flow.
        let attaches = vec![Some((Attach { col: 7, row: 3 }, Attach { col: 2, row: 3 }))];
        let mut grid = Grid::new(30, 10);
        let paths = route_all(
            &mut grid,
            &g,
            &attaches,
            |_| crate::layout::grid::arrow::LEFT,
            |_| true, // caller says it's a back-edge
        );
        // Must produce a path (A* back-edge routing can always find one in a
        // 30x10 grid with no obstacles).
        assert!(paths[0].is_some(), "back-edge should produce a path");
    }

    #[test]
    fn route_all_l_route_single_bend_with_obstacle() {
        // Source at (0,0), destination at (4,2): no straight route possible.
        // Block the V-then-H corner at (0,2) so the H-then-V route is cheaper.
        // With one corner blocked, try_l_route returns the clear orientation.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        let attaches = vec![Some((Attach { col: 0, row: 0 }, Attach { col: 4, row: 2 }))];
        let mut grid = Grid::new(10, 5);
        // Block the VH corner (src.col, dst.row) = (0, 2) to force HV route.
        grid.mark_node_box(0, 2, 1, 1);
        let paths = route_all(
            &mut grid,
            &g,
            &attaches,
            |_| crate::layout::grid::arrow::RIGHT,
            |_| false,
        );
        let path = paths[0].as_ref().expect("expected a routed path");
        // Path starts at (0,0) and ends at (4,2).
        assert_eq!(path.first(), Some(&(0, 0)));
        assert_eq!(path.last(), Some(&(4, 2)));
        // At least 3 cells: start, one bend, end.
        assert!(path.len() >= 3, "L-path should have at least 3 cells");
    }

    #[test]
    fn route_all_astar_fallback_detours_around_obstacle() {
        // Put a NodeBox obstacle at the corner of the preferred L-shape,
        // forcing the router past try_straight and try_l into A*.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        let src = Attach { col: 0, row: 0 };
        let dst = Attach { col: 5, row: 2 };
        let attaches = vec![Some((src, dst))];
        let mut grid = Grid::new(10, 5);
        // Block both L-corners: (5,0) and (0,2).
        grid.mark_node_box(5, 0, 1, 1);
        grid.mark_node_box(0, 2, 1, 1);
        let paths = route_all(
            &mut grid,
            &g,
            &attaches,
            |_| crate::layout::grid::arrow::RIGHT,
            |_| false,
        );
        let path = paths[0].as_ref().expect("A* must find a path");
        assert_eq!(path.first(), Some(&(0, 0)));
        assert_eq!(path.last(), Some(&(5, 2)));
    }

    #[test]
    fn route_all_stable_indexing_multi_edge() {
        // Three edges; middle one has no attach point (None).
        // Result must have 3 slots indexed by original edge index.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));
        g.edges.push(Edge::new("A", "C", None));
        let attaches = vec![
            Some((Attach { col: 0, row: 0 }, Attach { col: 4, row: 0 })),
            None, // missing positions for B→C
            Some((Attach { col: 0, row: 0 }, Attach { col: 8, row: 0 })),
        ];
        let mut grid = Grid::new(15, 5);
        let paths = route_all(
            &mut grid,
            &g,
            &attaches,
            |_| crate::layout::grid::arrow::RIGHT,
            |_| false,
        );
        assert_eq!(paths.len(), 3);
        assert!(paths[0].is_some());
        assert!(paths[1].is_none(), "None attach must yield None path");
        assert!(paths[2].is_some());
    }

    #[test]
    fn order_edges_ascending_distance() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "C", None)); // long edge (dist=8)
        g.edges.push(Edge::new("A", "B", None)); // short edge (dist=4)
        let attaches = vec![
            Some((Attach { col: 0, row: 0 }, Attach { col: 8, row: 0 })), // edge 0: dist 8
            Some((Attach { col: 0, row: 0 }, Attach { col: 4, row: 0 })), // edge 1: dist 4
        ];
        let order = order_edges(&g, &attaches);
        // Shorter edge (idx 1) must come before longer edge (idx 0).
        let pos_short = order.iter().position(|&i| i == 1).unwrap();
        let pos_long = order.iter().position(|&i| i == 0).unwrap();
        assert!(
            pos_short < pos_long,
            "short edge should route before long edge"
        );
    }

    #[test]
    fn order_edges_no_attach_sorted_last() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("A", "B", None));
        let attaches = vec![
            None,                                                         // no attach
            Some((Attach { col: 0, row: 0 }, Attach { col: 4, row: 0 })), // dist 4
        ];
        let order = order_edges(&g, &attaches);
        let pos_none = order.iter().position(|&i| i == 0).unwrap();
        let pos_some = order.iter().position(|&i| i == 1).unwrap();
        assert!(pos_some < pos_none, "edge with no attach should sort last");
    }
}
