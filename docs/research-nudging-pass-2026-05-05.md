# Post-Routing Nudging Pass — Implementation Research (2026-05-05)

**Audience:** One engineer with the codebase open. All file references use
absolute paths from the crate root. Bug labels match
`docs/scope-launch-quality-plan-2026-05-04.md`.

---

## Executive Summary

- **Our path data is already kept.** `route_all` in `router.rs` returns
  `Vec<Option<Vec<(usize, usize)>>>` indexed by edge index, and
  `render/unicode.rs:694` holds it alive in `paths` for the full post-routing
  loop. A nudging pass can consume `&mut paths` and `&mut grid` at exactly
  one insertion point: `unicode.rs:707` (the line immediately after
  `router::route_all` returns).

- **Bug 4 (corner displacement) is NOT called "nudging" in the literature.**
  It is part of libavoid's `nudgeOrthogonalSegmentsConnectedToShapes` option,
  which is modelled as a shape-buffer constraint, not a post-pass segment
  shift. For us it translates to a `NearNodeBox` cost band in A* — no
  separate pass required. The right name is "shape-buffer distance enforcement"
  or "obstacle-hugging avoidance."

- **Bug 5 (parallel-merge) IS the classic nudging operation.** The canonical
  name is "segment nudging" (Wybrow 2009 §4) or "edge nudging" (Hegemann &
  Wolff 2023). On a discrete character grid there is no known prior art for
  this; all published implementations (libavoid, Hegemann, graph-easy) operate
  on continuous pixel coordinates. We are inventing for the ASCII/cell domain.

- **Architectural surprise:** libavoid's nudging is a VPSC (separation-
  constraint LP) solver call — not greedy, not iterative in the traditional
  sense. On a 1-cell grid an equivalent pass can be implemented as a greedy
  left-to-right sweep of channel occupancy buckets without any LP. The
  `SAME_AXIS_COST` perimeter-bias approach already approximates this; a full
  nudging pass would add a post-routing scan over `paths` and re-stamp affected
  cells.

---

## libavoid's Nudging Algorithm — Implementation Specifics

### Repository

`github.com/mjwybrow/adaptagrams`, subtree `cola/libavoid/`.
Also maintained as a mirror in `github.com/mjwybrow/dunnart/libavoid/`
(used for direct source browsing; same algorithm).

### Connector path data structure

A routed connector is represented as a `ConnRef` object. Its visual geometry
is a `Polygon` (point array, `displayRoute()`), not a segment list. The nudging
pass works on `NudgingShiftSegment` objects, which are built from the polygon by
identifying maximal collinear runs of points in one dimension (`indexes` vector
holds the point indices into the polygon that bound each collinear segment).

```
ConnRef
  └── Polygon displayRoute()    ← array of (x,y) points
        each maximal collinear run → NudgingShiftSegment {
            connRef: &ConnRef,
            indexes: [low_pt_idx, high_pt_idx],
            minSpaceLimit / maxSpaceLimit: channel walls,
            variable: *Variable,    ← VPSC solver variable
            fixed: bool,
            sBend / zBend: bool,    ← topology shape flags
        }
```

The `sBend` flag is true when moving the segment in one direction would create
a new bend (S-shape: the segment is between two bends that are in opposite
directions). The `zBend` flag similarly marks Z-shaped topology. Both flags
constrain how far the VPSC solver is allowed to move the segment.

### Pass order in `router.cpp`

`Router::rerouteAndCallbackConnectors()` (line 924 in Inkscape's copy):

1. `connector->generatePath()` — A* routing for each connector (lines 934–979)
2. `m_hyperedge_rerouter.performRerouting()` — hyperedge adjustments (line 983)
3. `improveCrossings()` — crossing-penalty reroutes (line 986)
4. `m_hyperedge_improver.execute()` — optional hyperedge improvement (line 988)
5. **`improveOrthogonalRoutes(this)`** — nudging pass (line 999) ← final step
6. `performCallback()` — notify host application (lines 1013–1029)

Nudging is the last transformation before display. It runs on routes that are
already topology-correct; it only changes position within the feasible channel.

### `ImproveOrthogonalRoutes::execute()` structure

Source: `dunnart/libavoid/orthogonal.cpp` (dunnart mirror of adaptagrams).

```
ImproveOrthogonalRoutes::execute():
    simplifyOrthogonalRoutes()             // remove redundant bends
    buildConnectorRouteCheckpointCache()   // cache checkpoint metadata

    if performUnifyingNudgingPreprocessingStep and fixedSharedPathPenalty==0:
        for dim in [0, 1]:                 // X then Y
            buildOrthogonalNudgingSegments(dim, segmentList)
            buildOrthogonalChannelInfo(dim, segmentList)
            nudgeOrthogonalRoutes(dim, justUnifying=true)   // unify first

    for dim in [0, 1]:                     // X then Y
        buildOrthogonalNudgingSegments(dim, segmentList)
        buildOrthogonalChannelInfo(dim, segmentList)
        nudgeOrthogonalRoutes(dim, justUnifying=false)      // then nudge

    simplifyOrthogonalRoutes()             // re-simplify after nudge
    topologyImprovement()                  // optional topology cleanup
```

### `nudgeOrthogonalRoutes()` core loop

For each dimension (0=X-axis segments, 1=Y-axis segments):

1. **Extract regions:** Pop the front segment from `segmentList`; collect all
   segments that overlap it in the perpendicular dimension (same row or column,
   overlapping span) — this forms one "channel region."
2. **Sort** (skipped during unifying phase): `linsort()` orders segments within
   the region using a partial-order comparator (`CmpLineOrder`) that enforces
   crossing-minimisation (metro-line ordering). Incomparable segments are
   inserted via a deferred retry queue.
3. **Solver setup:** For each segment in the region, call
   `createSolverVariable()` which allocates a VPSC `Variable` with
   `desiredPosition = current position` and `weight = 1.0`. Fixed segments
   get tight `minSpaceLimit == maxSpaceLimit` constraints.
4. **Constraint construction:** For each adjacent pair in the sorted order,
   create a `Constraint(left->variable, right->variable, gap)` where
   `gap = idealNudgingDistance` (default 4 px in libavoid; 1 cell in our
   domain). Constraints also enforce the channel-wall limits.
5. **Solve:** `IncSolver(variables, constraints).solve()` — VPSC incremental
   block-based solver. Minimises `sum of (pos - desiredPos)^2` subject to
   separation constraints. Single-shot per region (not iterative).
6. **Apply:** `updatePositionsFromSolver()` reads `variable->finalPosition` and
   moves the polygon points for each segment.
7. **Loop:** Continue until `segmentList` is empty (all regions processed).

**Exit condition:** The outer loop terminates when all segments have been
assigned to a region and processed. There is no global iteration; each region
is processed exactly once per pass. If a re-simplification after nudging
produces new collinear runs, `simplifyOrthogonalRoutes()` handles them
separately.

### Invariants preserved

- **Topology:** S-bend and Z-bend flags prevent the VPSC solver from moving
  a segment past its adjacent bend points, which would change the route
  topology (number and direction of bends).
- **Endpoint connectivity:** Fixed-segment logic (`fixed = true` when the
  segment attaches to a shape) prevents the final leg from being repositioned
  away from the port.
- **Channel walls:** `minSpaceLimit` and `maxSpaceLimit` encode the obstacle
  boundaries; the VPSC constraints enforce them.

---

## ELK's Nudging Integration

ELK wraps libavoid via a JNI bridge. The three options map directly onto
libavoid's `RoutingOption` enum values:

| ELK option | libavoid enum value | Default |
|---|---|---|
| `nudgeOrthogonalSegmentsConnectedToShapes` | `0` | `false` |
| `nudgeOrthogonalTouchingColinearSegments` | `3` | `false` |
| `nudgeSharedPathsWithCommonEndPoint` | `6` | `true` |
| `idealNudgingDistance` | (parameter, not option) | `4` |

Sources:
- https://eclipse.dev/elk/reference/options/org-eclipse-elk-alg-libavoid-nudgeOrthogonalSegmentsConnectedToShapes.html
- https://eclipse.dev/elk/reference/options/org-eclipse-elk-alg-libavoid-nudgeOrthogonalTouchingColinearSegments.html
- https://eclipse.dev/elk/reference/algorithms/org-eclipse-elk-alg-libavoid.html

### What the three options actually control (from libavoid `router.h`)

These are three modes of the SAME nudging pass, not three separate passes.
They differ in which segment categories are included in `buildOrthogonalNudgingSegments`:

**`nudgeOrthogonalSegmentsConnectedToShapes` (default: false)**
When false, the "final segments" (legs that attach to a shape boundary) are
marked `fixed = true` in `NudgingShiftSegment`, preventing the VPSC solver from
moving them. Enabling this allows final segments to drift within the shape
buffer band — useful when ports are floating rather than fixed. In ELK's wrapper
the description notes: "Usually these segments are fixed, since they are
considered to be attached to ports." This is the operation closest to Bug 4's
corner-displacement concern.

**`nudgeOrthogonalTouchingColinearSegments` (default: false)**
Controls whether collinear segments that merely *touch* at an endpoint (zero-
length overlap) are nudged apart. The ELK docs note: "The overlap will usually
be resolved in the other dimension, so this is not usually required." This is
equivalent to the degenerate case where two routes share exactly one corner
point.

**`nudgeSharedPathsWithCommonEndPoint` (default: true)**
The main nudging operation. Separates the intermediate segments of connectors
that share a routing channel and converge on a common endpoint (a fork or join).
This is the operation that produces the "fan-out" appearance where several edges
exit a node in parallel and spread out as they travel away.

### Default behaviour without any options enabled

With `nudgeSharedPathsWithCommonEndPoint = true` and the others at `false`,
libavoid nudges shared-channel intermediate segments but leaves final port legs
fixed. This is the baseline that produces readable fan-outs at shared-endpoint
forks.

### Documented gotchas

No open ELK issue tracker entries were found that specifically document nudging
regressions. The ELK docs note two architectural constraints:

1. Nudging final segments (`nudgeOrthogonalSegmentsConnectedToShapes`) "will
   allow routes to be nudged up to the bounds of shapes" — i.e. it can push a
   connector all the way to the shape wall, which may look worse if the halo is
   small.
2. The unifying preprocessing step is only enabled when
   `fixedSharedPathPenalty == 0` (the default). Setting a shared-path penalty
   disables the pre-unification, which can leave co-directional segments
   overlapping before the main nudge pass runs.

---

## ASCII-Grid Analogues

### graph-easy (Perl, `github.com/ironcamel/Graph-Easy`)

The `lib/Graph/Easy/Layout.pm` module contains `_optimize_layout`, which
performs post-routing *compaction* (merging adjacent cells of the same type,
removing placeholder cells). It does NOT perform nudging, track assignment, or
segment separation. The model is cell-blocking: each cell holds at most one
edge segment, and A* simply avoids occupied cells. Parallel edges must find
distinct paths during routing; there is no post-routing redistribution.

This is architecturally identical to our current model: soft-obstacle costs
in A* (our `SAME_AXIS_COST`/`CROSS_AXIS_COST`) vs. hard cell-blocking
(graph-easy). Neither has a nudging pass.

### mermaid-cli / dagre / elkjs

These operate on pixel coordinates. Not relevant.

### Rust grid routers

Searches for Rust crates implementing post-routing nudging on character grids
returned no results. The `pathfinding` crate, `hierarchical_pathfinding`, and
`maze-routing` all implement path-finding only; none include a segment-
separation or track-assignment post-pass.

### VLSI track assignment

VLSI detailed routing literature (e.g., track assignment, rip-up-and-reroute)
works on a discrete grid but operates at the net level, not the diagram-
connector level. The scale, cost model, and topology constraints are
incompatible. Not a useful reference.

**Conclusion:** No published or open-source implementation of post-routing
nudging on a discrete character cell grid exists. All prior art is pixel-
continuous. We are inventing for the ASCII domain.

---

## The Two Operations — Names and Algorithms

### Corner Displacement (Bug 4)

**Name in literature:**
Not an independent named operation. It falls under two related concepts:

- **Shape buffer distance** (libavoid) — inflate each obstacle's bounding
  polygon by a constant before constructing the visibility graph. Routes are
  then forced to stay outside the inflated boundary, achieving clearance
  automatically during A* routing.
- `nudgeOrthogonalSegmentsConnectedToShapes` — the libavoid option that, when
  enabled, allows the A* leg *arriving at* a shape to be nudged away from the
  wall within the shape buffer band. This is a post-routing adjustment, not a
  routing-time constraint.

The symptom described as "Bug 4" (a corner glyph landing inside the halo of a
non-endpoint node) is in libavoid's model a routing failure, not a nudging
target: the shape buffer distance would prevent routing inside the halo in the
first place. In our model the equivalent fix is a `NearNodeBox` cost band in
A*, making halo cells expensive but not impassable, so A* routes around them
without requiring a post-pass.

**Reference:**
- Wybrow, Marriott, Stuckey (2009). GD'09. DOI: 10.1007/978-3-642-11805-0_22.
  Section 2 ("Shape buffer distance"); preprint at
  https://users.monash.edu/~mwybrow/papers/wybrow-gd-2009.pdf
- libavoid `router.h`, `shapeBufferDistance` parameter.
  https://www.adaptagrams.org/documentation/classAvoid_1_1Router.html

**Classification for our implementation:**
This is an A* cost-model change, not a separate pass. The "nudging pass"
framing does not apply to Bug 4. The correct implementation is:

1. After placing all node boxes (`mark_node_box`), mark a 1-cell ring around
   each box as `Obstacle::NearNodeBox` (new variant).
2. In `route_edge_with_inner_cost` add a cost branch for `NearNodeBox`:
   `step += NEAR_NODE_COST` where `NEAR_NODE_COST` is between `CROSS_AXIS_COST`
   (3.0) and `SAME_AXIS_COST` (10.0), e.g. 6.0.
3. Exempt the source and destination attach cells from the halo penalty.

**Minimum I/O:**
- Input: the existing `obstacles` grid (already has `NodeBox` cells).
- Output: a new `NearNodeBox` ring written into `obstacles` before routing
  begins. No path data needed; no post-pass.

### Parallel-Merge (Bug 5)

**Name in literature:**
- **"Segment nudging"** (Wybrow 2009 §4) — the canonical name for separating
  co-directional segments in shared channels.
- **"Edge nudging"** (Hegemann & Wolff 2023, arXiv:2309.01671) — same concept,
  broader framing.
- **"Constrained nudging"** vs. **"full nudging"** (Hegemann & Wolff 2023) —
  distinguish between nudging with fixed vertex positions vs. nudging with
  allowed vertex displacement.
- NOT "edge bundling" — that is the opposite operation (consolidating visually
  into one thick bundle). Our goal is *separation*, not consolidation.
- NOT "track merging" — that would merge two routes onto one line (reduce
  clutter), which is only valid when routes are semantically identical.
- NOT "channel consolidation" — that is a layout-level concept (compacting
  empty channels), not a routing post-pass.

The correct phrase for what Bug 5 needs is: **nudge apart co-directional
segments in shared routing channels.**

**Reference:**
- Wybrow et al. 2009. GD'09. Section 4 ("Nudging").
  DOI: 10.1007/978-3-642-11805-0_22.
- Hegemann & Wolff 2023. arXiv:2309.01671.
  https://arxiv.org/abs/2309.01671

The metro-line ordering problem cited in Wybrow 2009 as the ordering sub-step
is:
- Benkert, Nöllenburg, Uno, Wolff (2006). "Minimizing intra-edge crossings in
  wiring diagrams and public transportation maps." GD 2006.

**Minimum I/O for a discrete-grid nudge pass:**

```
Input:
  paths: &mut Vec<Option<Vec<(usize, usize)>>>
    — the Vec<(col, row)> paths returned by route_all, BEFORE stamping glyphs
      (currently paths are stamped DURING routing in draw_routed_path — see
      codebase note below)
  grid: &mut Grid
    — the current obstacle + glyph canvas

Output:
  Modified paths Vec (some paths have cells shifted by 1 row or 1 col)
  Grid re-stamped to match the modified paths

Algorithm (greedy scan, no LP required at grid scale):
  1. Build a channel map: for each row r, collect all (edge_idx, col_start,
     col_end) spans of horizontal segments; for each col c, collect all
     (edge_idx, row_start, row_end) spans of vertical segments.
  2. For each channel with 2+ co-directional segments (same row or col,
     overlapping span):
       a. Sort segments by their perpendicular coordinate (pre-nudge position).
       b. Check: do any two segments currently occupy the SAME row/col? If so,
          try to shift one by 1 cell (the SAME_AXIS direction) to a free row/col.
       c. If the shifted path is obstacle-free (no NodeBox or NearNodeBox), apply
          the shift: erase old cells, restamp new cells.
  3. Repeat until no more improvements (convergence in 1–2 passes on typical
     graphs — channels are small).
```

---

## Codebase Capability Check

### Path data persistence: KEPT

`route_all` in `crates/mermaid-text/src/layout/router.rs:56` returns
`Vec<Option<Vec<(usize, usize)>>>`. The caller in
`crates/mermaid-text/src/render/unicode.rs:694` stores this in `let paths`
and uses it throughout the post-routing loop (lines 717–900+) for: tip
overwriting, path style overdraw, label placement, and back-tip placement.

The paths are live in `unicode.rs` from line 694 to beyond line 900.

**Critical caveat:** Paths are stamped onto the grid *during* routing, inside
`Grid::draw_routed_path` (called from `route_edge_with_inner_cost:1541`). This
means by the time `route_all` returns, every path is already drawn on the grid
and the obstacle map is already updated. A nudging pass that moves a path must:
1. Erase the old cells from the grid (clear glyphs + clear
   `EdgeOccupied*` obstacle flags).
2. Re-stamp the new cells.

This is more expensive than the libavoid model (which nudges before rendering),
but it is feasible: the grid's direction-bit canvas stores enough information
to reverse a stamped path.

### Refactor seam location

**Exact insertion point:**

```
crates/mermaid-text/src/render/unicode.rs:707
```

This is the line immediately after `router::route_all` returns `paths`. At
this point: all routes are stamped on the grid, `paths` holds the full
coordinate vectors, all node boxes are marked, but label placement has not yet
occurred (labels are collected into `pending_labels` and flushed later). This
is the right seam for a nudging pass.

The call would look like:

```rust
// line 706 — let paths = router::route_all(...);
// INSERT HERE:
nudge_pass::run(&mut grid, &mut paths, &attach_points, &graph);
// line 708 — begin the per-edge post-processing loop
```

The `nudge_pass` module would live at:
`crates/mermaid-text/src/layout/nudge.rs` (parallel to `router.rs`).

### Existing batch patterns to mirror

**`sg_max_width_cache` in `layered.rs:1335`**

This is the canonical batch-collected-then-applied pattern in this codebase.
Pattern structure:
- Collect data about all items in a pre-pass (fill a `HashMap<String, usize>`).
- Apply the collected data in the main loop.
- Use `.entry(...).or_insert_with(...)` for lazy population.

A nudging pass would use a similar two-phase structure:
- Phase A: scan `paths` to build a `ChannelMap` (per-row and per-col occupancy
  lists).
- Phase B: iterate `ChannelMap` entries with 2+ occupants, compute shifts,
  apply to `paths` and `grid`.

**`pending_labels` in `unicode.rs:711`**

The label-placement deferral pattern: collect mutations during the first loop,
apply after the loop. A nudging pass that modifies paths would follow the same
discipline — don't partially re-stamp paths inside the scan; collect all
desired shifts first, then apply in a second pass to avoid move-A-collides-
with-B races.

**`prior_path_cells_by_pair` in `unicode.rs:714`**

A `HashMap<(&str, &str), HashSet<(usize, usize)>>` tracking which cells belong
to which edge pair, used for parallel-edge label placement. A nudging pass
would naturally re-use this structure or a similar per-edge cell set.

---

## Recommended Implementation Order

- **Bug 4 first (A* cost-model change, no separate pass)**
  - Add `Obstacle::NearNodeBox` variant to `grid.rs` Obstacle enum.
  - In `Grid` setup (called before routing), mark 1-cell ring around each
    `NodeBox` region as `NearNodeBox`, skipping the known attach cells.
  - In `route_edge_with_inner_cost`, add cost branch for `NearNodeBox` at
    ~6.0 (between `CROSS_AXIS_COST` and `SAME_AXIS_COST`).
  - Run gallery check; tune the cost constant.
  - This is entirely within `grid.rs`; no architecture change.

- **Bug 5 approximation (already scoped, lowest-risk step)**
  - The perimeter-bias `SAME_AXIS_COST` reduction for back-edges described in
    the prior research doc. Already scoped in scope-launch-quality-plan. This
    alone may be sufficient for the launch gallery.

- **Bug 5 full nudging pass (if approximation is insufficient)**
  - New `crates/mermaid-text/src/layout/nudge.rs` module.
  - Insert call at `unicode.rs:707`.
  - Implement greedy channel-scan (no LP needed at this scale).
  - Requires: grid erase-and-restamp capability (new helper in `grid.rs`).
  - Requires: channel map data structure (internal to `nudge.rs`).
  - Run gallery check after implementation.

---

## Open Questions

1. **The PDF papers are unreadable as binary.** Section 4 of Wybrow 2009
   (the full LP formulation and separation-constraint details) could not be
   read directly — the PDF binary was not decoded by the fetch tool. The
   algorithmic description above is reconstructed from: (a) the libavoid
   source code structure, (b) Hegemann & Wolff 2023 abstract, (c) the
   Inkscape doxygen-rendered source. If the exact LP formulation matters for
   correctness verification, obtain the paper via DOI:
   10.1007/978-3-642-11805-0_22 or the preprint URL:
   https://users.monash.edu/~mwybrow/papers/wybrow-gd-2009.pdf.

2. **Erase-and-restamp capability.** The nudging pass must erase old path
   cells and re-stamp new ones. The grid currently has `draw_routed_path`
   (draw) and `overdraw_path_style` (restyle) but no "erase path" function.
   Before implementing a nudging pass, add `Grid::erase_path(path: &[(usize,
   usize)])` that: (a) clears the glyph to `' '`, (b) resets
   `EdgeOccupied*` obstacle flags, (c) clears direction bits. This must be
   careful not to erase cells that belong to a DIFFERENT edge's path. A
   per-cell edge-occupancy counter (or reference count) would be needed if
   paths can share junction cells.

3. **Junction cell ownership.** Two paths sharing a `┼` crossing cell both
   wrote into it. Erasing one path should degrade `┼` → the other path's
   glyph, not blank the cell. The current direction-bit canvas supports this
   (subtract the erased path's direction bits and recompute the glyph), but
   the `EdgeOccupied*` obstacle flags do not have a reference count. This is
   the hardest correctness problem for the nudge pass and should be designed
   before any code is written.

4. **One-cell grid granularity.** libavoid's `idealNudgingDistance = 4`
   separates segments by 4 pixels. On a character grid "4 cells apart" means
   4 character widths of whitespace between routes — too wide for typical
   flowcharts. The correct discrete analogue is `idealNudgingDistance = 1`
   (one cell apart). The nudge pass should aim to ensure at minimum 1 free
   cell between co-directional parallel routes.

5. **Is the SAME_AXIS_COST approximation sufficient?** This question can only
   be answered by running `scripts/render-gallery.sh` after implementing the
   Bug 4 halo change and checking whether adjacent parallel routes still appear
   in the gallery. If Bug 4's halo already pushes routes off box walls and the
   `SAME_AXIS_COST` already discourages same-row sharing, the full nudging pass
   may not be needed for launch quality. Measure before building the pass.

6. **Hegemann & Wolff 2023 full nudging vs. constrained nudging distinction.**
   The abstract distinguishes "constrained nudging" (vertex positions fixed)
   from "full nudging" (vertex positions can shift with minimum box distances).
   For our use case vertex positions are always fixed (we never move node boxes
   after layout), so only constrained nudging is relevant. This is also
   libavoid's `nudgeSharedPathsWithCommonEndPoint` default mode. Confirming
   this distinction from the full paper (arXiv:2309.01671) would be useful
   before designing the discrete analogue.

---

## Primary Sources Cited

- Wybrow, Marriott, Stuckey (2009). "Orthogonal Connector Routing." GD'09,
  LNCS 5849. DOI: 10.1007/978-3-642-11805-0_22.
  Preprint: https://users.monash.edu/~mwybrow/papers/wybrow-gd-2009.pdf

- Hegemann, Wolff (2023). "A Simple Pipeline for Orthogonal Graph Drawing."
  GD'23, Springer LNCS. arXiv:2309.01671.
  https://arxiv.org/abs/2309.01671

- libavoid `router.h` (nudging options, VPSC parameter defaults):
  https://www.adaptagrams.org/documentation/router_8h_source.html

- libavoid `orthogonal.cpp` (dunnart mirror — function structure):
  https://github.com/mjwybrow/dunnart/blob/master/libavoid/orthogonal.cpp

- libavoid `vpsc.h` (Variable, Constraint, IncSolver class definitions):
  https://raw.githubusercontent.com/mjwybrow/adaptagrams/master/cola/libavoid/vpsc.h

- libavoid call order (Inkscape doxygen `router.cpp` source):
  https://inkscape.gitlab.io/inkscape/doxygen/router_8cpp_source.html

- ELK libavoid algorithm reference (three nudging options + defaults):
  https://eclipse.dev/elk/reference/algorithms/org-eclipse-elk-alg-libavoid.html

- graph-easy `Layout.pm` (no nudging, cell-blocking model confirmed):
  https://github.com/ironcamel/Graph-Easy/blob/master/lib/Graph/Easy/Layout.pm

- Our codebase seam:
  `crates/mermaid-text/src/render/unicode.rs:694` (route_all call)
  `crates/mermaid-text/src/layout/router.rs:56` (route_all definition)
  `crates/mermaid-text/src/layout/grid.rs:1608` (draw_routed_path)
