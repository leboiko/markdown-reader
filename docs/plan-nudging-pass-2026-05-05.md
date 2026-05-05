# Implementation Plan — Post-Routing Nudging Pass (2026-05-05)

**Note on delivery**: The task asked for a markdown file at `docs/plan-nudging-pass-2026-05-05.md`, but the planning environment is strictly read-only (no Write/Edit tools available). The plan is delivered inline below. The parent agent is expected to write it to disk verbatim if persistence is desired; the content is structured as a self-contained markdown document.

---

## Scope ceiling

Hard upfront limits. Any single one tripped → revert + document recipe.

**Code-line ceiling:**
- `crates/mermaid-text/src/layout/grid.rs`: ≤ 80 net new lines (mostly `erase_path` + helpers).
- `crates/mermaid-text/src/layout/nudge.rs`: ≤ 350 lines including doc comments and module-internal tests.
- `crates/mermaid-text/src/layout/mod.rs`: 1-line module declaration.
- `crates/mermaid-text/src/render/unicode.rs`: ≤ 10 net new lines (the call-site + any `pub(crate)` re-exports needed for the channel-map types).
- **Hard total ceiling: ≤ 450 net new lines across the four files.**

**Snapshot churn ceiling:**
- ≤ 60 of 116 files in `crates/mermaid-text/tests/snapshots/` (matches research §"Open Question #5" estimate of 50-100; we land on the conservative half because Bug 4's halo is only triggered by fan-in topologies and Bug 5 only by multiple back-edges in the same diagram).
- **0 Bucket-C (regression) snapshot diffs**, classified per `feedback_harness_scope_ceilings.md`. A single regression diff ⇒ revert.
- Crossings test (`tests/crossings.rs`) baseline must not regress by > +10%; absolute total must not increase by more than 5 across the whole 19-fixture suite.

**Test-count ceiling:**
- New tests added: ≤ 6 (two un-`#[ignore]`d acceptance tests + four module-internal `nudge.rs` unit tests + at most one new integration regression guard).
- Tests reclassified `#[ignore]` → `#[test]`: exactly 2 (the bugs we're fixing).
- Pre-existing `#[ignore]`d tests we MUST NOT break: `final_state_renders_at_rightmost_column` (was previously `#[ignore]`d, now FIXED per CHANGELOG; verify it still passes).

**Files-touched ceiling:**
- 4 source files (listed above) + 1 new module file. CHANGELOG.md and the scope doc count as housekeeping (always).
- A scope doc at `docs/scope-nudging-pass-2026-05-05.md` is mandatory by the project standard if anything is reverted; **create it pre-emptively and append phase-by-phase status**.

**Abort + document recipe** (when a ceiling trips):
1. Capture `git diff` and `cargo test --workspace 2>&1` output to a scratch file.
2. `git restore .` the source changes (keep CHANGELOG/scope-doc edits).
3. Append a new "Attempt N — reverted" subsection to `docs/scope-nudging-pass-2026-05-05.md` with:
   - What ceiling tripped (lines, snapshots, regressions, time).
   - The hypothesis being tested.
   - The exact failure mode (test name, snapshot diff names, observed behaviour).
   - Why this hypothesis is wrong / what the next attempt should do differently.
4. Re-`#[ignore]` the bug tests (they were never un-ignored if the attempt didn't land).
5. **Do not partially commit.** A half-broken nudge pass is worse than no pass; per `MEMORY.md` rule 16: "a documented 'no' beats a half-broken 'yes'."

---

## Architecture overview

### Module layout

```
crates/mermaid-text/src/layout/
  grid.rs       — Grid struct + EdgeOccupied* + draw_routed_path + NEW: erase_path
  router.rs     — route_all (unchanged)
  nudge.rs      — NEW: post-routing nudging pass
  mod.rs        — adds `pub(crate) mod nudge;`
```

### Public API surface (additions)

`crates/mermaid-text/src/layout/grid.rs`:

```rust
/// Erase a previously-drawn path's cell contributions. After erasing, the cells
/// retain any other paths' direction bits and re-render with the surviving
/// glyph. Tip cells are unprotected, glyph reset, and EdgeOccupied flags
/// cleared (because tips are never shared between paths — they're protected).
pub(crate) fn erase_path(&mut self, path: &[(usize, usize)]);

/// Return the obstacle classification at (col, row). Internal helper used by
/// the nudge pass to verify candidate destination cells are routable.
pub(crate) fn obstacle_at(&self, col: usize, row: usize) -> ObstacleKind;

/// Public-facing copy of the internal Obstacle enum (or alternatively, expose
/// is_free / is_node_box / is_edge_occupied predicates). Decision: see "Risk
/// register / API surface".
pub(crate) enum ObstacleKind { Free, InnerArea, NodeBox, EdgeH, EdgeV }
```

`crates/mermaid-text/src/layout/nudge.rs`:

```rust
/// Run the nudging pass. Mutates paths in place and re-stamps grid.
pub(crate) fn run(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    attach_points: &[Option<(Attach, Attach)>],
    graph: &Graph,
    edge_is_back_flags: &[bool],
    tip_for: impl Fn(usize) -> char,
);

// Module-internal:
struct ChannelMap { ... }     // per-row, per-col span occupancy
struct Segment { edge: usize, axis: Axis, fixed_coord: usize, range: (usize, usize), endpoints: (Endpoint, Endpoint) }
enum Endpoint { Free, AttachedToNode, AttachedToCorner }
fn build_channel_map(paths) -> ChannelMap;
fn plan_corner_displacements(grid, paths, graph, attach_points) -> Vec<Shift>;
fn plan_parallel_merges(channel_map, grid) -> Vec<Shift>;
fn apply_shifts(grid, paths, shifts, tip_for);
```

### Integration points (with file:line refs)

1. **`crates/mermaid-text/src/render/unicode.rs:706`** — current `route_all` call ends here. The nudge pass call inserts on a new line **at 707** (before the post-routing per-edge loop at 717). Concrete shape:

   ```rust
   let paths = router::route_all(...);  // existing line 694–706
   nudge::run(&mut grid, &mut paths, &attach_points, graph, &edge_is_back_flags,
              |idx| tip_char_for_edge(idx, graph.direction, &edge_is_back_flags));  // NEW line 707
   // line 708 onward: existing post-routing per-edge loop continues unchanged
   ```

   A small refactor pulls the tip-character closure out so both `route_all` and `nudge::run` share it (avoid duplication).

2. **`crates/mermaid-text/src/layout/mod.rs`** — add `pub(crate) mod nudge;`.

3. **`crates/mermaid-text/src/layout/grid.rs:1608`** — `draw_routed_path` is the inverse of the new `erase_path`. They must share a single helper (or be expressed as one bidirectional `stamp(path, op: Stamp | Erase)` function) so the bit semantics stay symmetric. Decision: keep them as two functions but factor the bit-derivation pass into a shared private `path_cell_bits(path) -> Vec<(col, row, bits)>` helper.

4. **`crates/mermaid-text/src/render/unicode.rs:879-932`** — back-edge join stamping (`back_edge_border_joins`, `back_edge_path_joins`) runs AFTER the nudge pass per the call order (it's in pass 2a.5, line 861+). This is critical: nudging may shift a back-edge path, but the `back_edge_path_joins` lookup is computed **at line 685** (before routing) using `border_cells`. If the nudge pass moves the back-edge's first path cell off the recorded `(col, row)` in `back_edge_path_joins`, the `┴`/`┘`/`└` exit-stub stamp lands in the wrong place.

   **Mitigation**: the nudge pass MUST NOT shift the source-attach cell or the cell adjacent to it (the cell that receives the exit-stub stamp). This is encoded as `Endpoint::AttachedToNode` on the path's first segment — segments with that endpoint type are pinned (cannot shift along their fixed-coordinate axis at the attached end, but interior cells can shift if the segment is long enough that the shift doesn't propagate to the endpoint).

5. **`crates/mermaid-text/src/render/unicode.rs:799-841`** — label placement runs AFTER the nudge pass (it's inside the per-edge loop at line 717-842). Labels are computed against the modified `paths`, so they automatically track shifted geometry. **No code change needed for label re-placement.** This is the architectural win of placing the nudge pass at line 707.

6. **`crates/mermaid-text/src/render/unicode.rs:692-688`** — `back_edge_border_joins` and `back_edge_path_joins` are populated BEFORE routing. The path-join entries point at the cell immediately below (LR) or right of (TD) the source border. After nudging, the back-edge's first interior cell may have moved; we MUST re-compute these from the post-nudge `paths[edge_idx]`. Concretely: replace the `back_edge_path_joins.push(sp)` at line 687 with a deferred lookup that runs AFTER `nudge::run` and reads from `paths[edge_idx][1]` (the first cell after the source attach). This is a small refactor inside `unicode.rs` but it's load-bearing for the `┴` exit stub.

### Data flow

```
attach_points (from compute_spread_attaches, line 685ish)
         │
         ▼
  route_all (router.rs:56)  ──→  paths: Vec<Option<Vec<(c,r)>>> + grid stamped
         │
         ▼
  nudge::run (NEW)  ──→  paths' (mutated) + grid' (re-stamped after erase+restamp)
         │
         ▼
  back_edge_path_joins re-derived from paths' (NEW)
         │
         ▼
  per-edge post-processing loop (unicode.rs:717+)  ──→  labels, styles, tip overrides
         │
         ▼
  pass 2a.5 back-edge stamps  ──→  ┴ ┘ └ exit stubs
         │
         ▼
  pass 2b labels, pass 3 node labels, render
```

---

## Phase A — Reproduction tests (re-validate)

**Goal**: confirm the two `#[ignore]`d tests are still failing the way the comments claim, and add stronger trap-checks. No source changes.

**Specific edits:**

1. `crates/mermaid-text/src/render/unicode.rs:3729` — `#[test] #[ignore = "..."] fn back_edges_share_return_corridor()` — run with `cargo test back_edges_share_return_corridor -- --include-ignored`. **Expected**: assertion fails, `height = 11 > 9`. Trap-check: `box_count == 5` already covers no-op renders. Add an additional assertion before the height check:

   ```rust
   // Strong trap-check: the diagram must contain TWO back-edges. A no-op
   // render that drops back-edges entirely passes the height bound.
   let arrow_back_count = lines.iter()
       .filter(|l| l.contains('◂') || l.contains('▴') || l.contains('◀') || l.contains('▾'))
       .count();
   assert!(arrow_back_count >= 2,
       "trap-check: expected ≥2 back-edge tips; found {arrow_back_count}");
   ```

   This guards against a regression where the nudge pass accidentally drops a path during shift-apply.

2. `crates/mermaid-text/src/render/unicode.rs:3884` — `#[test] #[ignore = "..."] fn route_corners_clear_non_endpoint_node_halos()` — same approach: add a strong trap-check that the A → Z, B → Z, C → Z, D → Z routes all exist by checking for arrow tips at Z's left edge:

   ```rust
   let z_box_left = ... // find │ Z │ left col
   let arrows_into_z = lines.iter()
       .filter(|l| l.get(z_box_left.checked_sub(1)?).map(|c| *c == '▶' || *c == '◀').unwrap_or(false))
       .count();
   assert!(arrows_into_z >= 4, "trap-check: not all 4 source routes reach Z");
   ```

   This guards against the trivial pass mode where a route fails to reach Z (no halo to pierce).

3. **Do NOT un-`#[ignore]` yet.** That happens at the end of Phase E.

**Success criteria:**
- `cargo test back_edges_share_return_corridor -- --include-ignored` → **fail** with the documented `height > 9` assertion.
- `cargo test route_corners_clear_non_endpoint_node_halos -- --include-ignored` → **fail** with the documented halo-glyph collision assertion.
- Both new trap-checks pass on the current (pre-fix) render — they only catch broken-render impostors, not the bug itself.
- `cargo test --workspace` baseline green.

**Estimated effort:** 0.25 sessions (≤ 1 hour).

---

## Phase B — `Grid::erase_path` infrastructure

**Goal**: implement the inverse of `draw_routed_path` so a nudged path can be cleanly removed before the new path is stamped.

**Specific edits:**

1. **`crates/mermaid-text/src/layout/grid.rs`** (around line 1608, near `draw_routed_path`):

   Add `pub(crate) fn erase_path(&mut self, path: &[(usize, usize)])`. Algorithm:

   For each cell `(c, r)` in `path`:

   a. **Compute the bits this path contributed** to the cell. Mirror `draw_routed_path`'s logic:
      - Interior cell (i ∈ [1, len-1)): bits = `neighbor_bit(c, r, prev) | neighbor_bit(c, r, next)`.
      - Tip cell (i = len-1): bits = `neighbor_bit(c, r, prev)` only — the tip isn't a junction, but the bit derivation is symmetric.
      - First cell (i = 0): bits = `neighbor_bit(c, r, next)`.

   b. **Subtract those bits** from `self.directions[r][c]` using `& !bits`.

   c. **Recompute the glyph** from the surviving direction bits via `recompute_cell_glyph(c, r)` (already exists at `grid.rs:481`).

   d. **Tip cell special-case**: tips are protected (`grid.rs:1657`). Erasing a tip means:
      - `unprotect_cell(c, r)` (line 469).
      - Set glyph to `' '` if surviving direction bits == 0, else `recompute_cell_glyph`.
      - **Do not** clear the tip from another path's surviving bits — only this path's tip bit is subtracted.
      - **Edge case**: if two paths share a tip cell (this should be impossible — each path has its own destination — but defensively assert in debug builds).

   e. **Obstacle flag downgrade** — the hardest correctness problem. See dedicated section below. Briefly: leave `EdgeOccupied*` set (binary flag, see decision below).

2. **Test** in `grid.rs` `mod tests` block:

   ```rust
   #[test]
   fn erase_path_clears_isolated_segment() {
       let mut g = Grid::new(10, 5);
       let path = vec![(2, 2), (3, 2), (4, 2), (5, 2)];
       g.draw_routed_path(&path, '▶');
       assert_eq!(g.get(3, 2), '─');
       g.erase_path(&path);
       assert_eq!(g.get(3, 2), ' ');
       assert_eq!(g.get(5, 2), ' ');  // tip cleared
   }

   #[test]
   fn erase_path_preserves_shared_junction() {
       let mut g = Grid::new(10, 5);
       let h_path = vec![(1, 2), (2, 2), (3, 2), (4, 2)];
       let v_path = vec![(2, 0), (2, 1), (2, 2), (2, 3)];
       g.draw_routed_path(&h_path, '▶');
       g.draw_routed_path(&v_path, '▼');
       // junction at (2, 2) — original glyph depends on tip vs interior.
       g.erase_path(&h_path);
       // After erasing horizontal, the vertical contribution survives.
       assert_eq!(g.get(2, 1), '│');  // pure vertical segment unchanged
       // (2, 2) now has UP bit only from v_path interior (next is (2,3) DOWN)
       // wait — (2,2) is interior of v_path (i=2 of 4), so v_path bits are UP|DOWN.
       // After h_path erase, (2,2) should be '│'.
       assert_eq!(g.get(2, 2), '│');
   }

   #[test]
   fn erase_path_handles_tip_unprotect() {
       let mut g = Grid::new(10, 5);
       let path = vec![(2, 2), (3, 2), (4, 2)];
       g.draw_routed_path(&path, '▶');
       // (4, 2) is tip — protected
       g.erase_path(&path);
       assert_eq!(g.get(4, 2), ' ');
       // Verify cell is no longer protected by trying to add_dirs.
       g.add_dirs(4, 2, DIR_LEFT | DIR_RIGHT);
       assert_eq!(g.get(4, 2), '─');
   }
   ```

**Success criteria:**
- `cargo test grid::tests::erase_path` → all 3 unit tests green.
- `cargo test --workspace` no new failures.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --all -- --check` clean.

**Estimated effort:** 0.5 sessions (≤ 2 hours). The function itself is ≤ 30 lines; the tests are the bulk of the work.

---

## Phase C — Bug 5 nudging (parallel-merge)

**Goal**: implement the channel-map scan that detects two parallel back-edges occupying adjacent rows (or columns) on a shared perimeter span and shifts one onto the other's row, producing a single shared corridor.

**Specific edits:**

1. **Create `crates/mermaid-text/src/layout/nudge.rs`**.

2. **Add `pub(crate) mod nudge;` to `crates/mermaid-text/src/layout/mod.rs`**.

3. **Add the `nudge::run` call at `crates/mermaid-text/src/render/unicode.rs:707`**, gated initially behind a constant `const NUDGE_PASS_ENABLED: bool = true;` for fast revert (delete the constant at end of Phase E if all green).

4. **Algorithm (Phase C scope: parallel-merge only):**

   a. **Build channel map** — scan all `paths`. For each segment (maximal collinear run of cells in one axis):
      - If horizontal (constant row, varying col): record `(edge_idx, axis=H, fixed=row, range=(col_min, col_max), endpoints)`.
      - If vertical: symmetric.
      - Endpoint metadata: `EndpointKind::AttachedToNode` if the segment touches `attach_points[edge_idx].0` or `.1`; `EndpointKind::Bend` if the next/prev cell exists in the same path; `EndpointKind::Tip` if it's the destination tip.

   b. **Group by axis + range overlap** — bucket segments by axis. Within each axis, two segments are **comparable** if they share a span (both horizontal AND `range_a` overlaps `range_b` AND `|fixed_a - fixed_b| <= MAX_NUDGE_DISTANCE`). The Bug 5 fixture has back-edges on adjacent perimeter rows so the typical distance is 2 (one empty row between corridors); `MAX_NUDGE_DISTANCE = 3` is conservative.

   c. **For each comparable pair, plan a merge**:
      - Source segment (the one to shift): pick the back-edge with the **shorter horizontal/vertical extent** (minimises ripple). Prefer back-edges over forward edges (`edge_is_back_flags[idx]` is true).
      - Target row/col: the other segment's `fixed` coordinate.
      - **Feasibility check**: simulate the shift. For each cell `(c, target_row)` in the new path, check `grid.obstacle_at(c, target_row)` — if any cell is `NodeBox`, abort this shift (mark as `Skipped::WouldHitNodeBox`).
      - **Bend-cost check**: the shift only changes the segment's `fixed_coord`. Adjacent bends must move with it. If the segment is bookended by two perpendicular segments, those perpendiculars must extend by `delta = old_fixed - new_fixed` cells. Check the extension cells are `Free` or `EdgeOccupied*` (acceptable cross). NodeBox encountered along the extension ⇒ abort.
      - **Endpoint pinning**: if either endpoint of the segment is `EndpointKind::AttachedToNode`, the segment cannot move along its perpendicular axis at all (would detach). For Bug 5 the relevant segments are the perimeter-corridor horizontals which have both endpoints as `Bend` (the corridor is between two corner bends), so this restriction doesn't fire. Add an explicit `if endpoint == AttachedToNode { skip; }` guard nonetheless — defends against future fixtures.

   d. **Apply shifts in batched fashion** (mirror the `pending_labels` pattern at `unicode.rs:711`):
      - First pass: collect all `Shift { edge_idx, old_path, new_path }` into a vector.
      - Second pass: for each shift, `grid.erase_path(&old_path)`, then `grid.draw_path(new_path.clone(), tip_for(edge_idx))`, then `paths[edge_idx] = Some(new_path)`.
      - **Why two-pass**: a single-pass apply could race when two shifts target the same destination row. Collecting first, then applying in order of decreasing `delta` avoids "shift A onto target → shift B onto same target → collision" by detecting in-progress destinations during the apply step.

5. **Constants** (in `nudge.rs`):

   ```rust
   const MAX_NUDGE_DISTANCE: usize = 3;       // max cells a segment can shift
   const MIN_SEGMENT_LEN_FOR_NUDGE: usize = 3; // don't nudge stub segments
   const IDEAL_SEPARATION: usize = 1;          // grid analogue of libavoid's 4px
   ```

6. **Module unit tests** (in `nudge.rs` `mod tests`):

   ```rust
   #[test]
   fn channel_map_detects_two_parallel_horizontals() { ... }
   #[test]
   fn parallel_merge_with_no_obstacle_between_succeeds() { ... }
   #[test]
   fn parallel_merge_blocked_by_node_box_aborts() { ... }
   #[test]
   fn endpoint_attached_to_node_is_pinned() { ... }
   ```

   Each test uses a hand-constructed `Grid` + synthetic `paths` to verify the channel-map scan + shift logic without going through the full `render()` pipeline.

7. **Acceptance test (un-`#[ignore]`)**:
   - At end of Phase C, run `cargo test back_edges_share_return_corridor -- --include-ignored`. Expected: pass.
   - Once it passes, **remove** the `#[ignore = "..."]` line at `unicode.rs:3729`. The test moves into the regular test suite.

**Success criteria:**
- `cargo test --workspace` adds 4 new green tests, plus `back_edges_share_return_corridor` flips from ignored to green.
- `cargo test --test crossings` baseline preserved (≤ +5 absolute crossings across 19 fixtures).
- `cargo test --test regression_corpus` Bucket-C count = 0.
- Snapshot churn ≤ 30 files (Bug 5 alone is the smaller of the two — affects only diagrams with ≥ 2 back-edges in shared corridors).

**Estimated effort:** 1.5 sessions (≤ 6 hours). Includes the channel-map data structure, segment classification, feasibility checks, and the batched-apply discipline.

---

## Phase D — Bug 4 corner displacement

**Goal**: detect path corners that landed in non-endpoint node halos (cells in the 1-ring around a NodeBox that aren't the path's source or destination attach point) and shift the corner outward by 1 cell.

**Specific edits:**

1. **Add a second scan to `nudge.rs`** — `plan_corner_displacements`. Reuses Phase C's `apply_shifts` infrastructure (this is the architectural win the user asked for in the prompt: "Phase D — corner-displacement variant — same module, different scan").

2. **Algorithm (Phase D scope: corner displacement only):**

   a. **Pre-compute halo set** — for each NodeBox cell `(nc, nr)`, mark the 8 surrounding cells as "halo of node X". A cell can belong to multiple node halos (corners between two boxes); represent as `HashMap<(col, row), Vec<NodeId>>`.

   b. **Scan all paths for corner cells** — a corner is any path cell with both H and V direction bits set (interior cell where the path bends). Iterate path cells; corner is at index `i` where `path[i-1]` and `path[i+1]` are perpendicular relative to `path[i]`.

   c. **For each corner cell**:
      - If the corner is in a halo of some node X, AND X is not an endpoint of this edge (`graph.edges[edge_idx].from != X.id` AND `graph.edges[edge_idx].to != X.id`), AND the path has length > 4 (no nudging tiny stubs):
      - Plan a shift: try moving the bend 1 cell further from X. Two candidate moves (the bend can shift either along its incoming axis or its outgoing axis to stay outside the halo); pick the one that produces a shorter total path length post-shift.
      - **Feasibility**: same as Phase C — every new cell must be `Free` or `EdgeOccupied*` (cross is OK), never `NodeBox` and never another node's halo.

   d. **Critical exemption (the subtle correctness problem from prior session)**: the source attach + the cell immediately adjacent to it must remain in the source's halo (because they ARE the source's halo by definition — the attach point sits on the box border, the next cell sits in the halo). Same for destination. Encode as: when computing "is this corner in a non-endpoint halo," exclude any halo cell that is `path[0]`, `path[1]`, `path[len-2]`, or `path[len-1]`. This was the convention the previous A* attempt missed.

   e. **Apply**: same `apply_shifts` batching as Phase C.

3. **Acceptance test (un-`#[ignore]`)**:
   - Run `cargo test route_corners_clear_non_endpoint_node_halos -- --include-ignored`. Expected: pass.
   - Remove `#[ignore = "..."]` at `unicode.rs:3884`.

4. **Module unit tests:**

   ```rust
   #[test]
   fn corner_in_non_endpoint_halo_is_shifted() { ... }
   #[test]
   fn corner_in_endpoint_halo_is_pinned() { ... }
   #[test]
   fn corner_shift_blocked_by_second_node_aborts() { ... }
   ```

**Can Phase D share Phase C's infrastructure?**

**Mostly yes, with two new pieces:**
- `apply_shifts(grid, paths, shifts, tip_for)` is shared verbatim (the apply step doesn't care whether the shift came from a parallel-merge plan or a corner-displacement plan).
- `Grid::erase_path` is shared.
- `obstacle_at` predicate is shared.
- **New for Phase D**: the halo-set pre-compute + the corner-detection scan + the endpoint-exemption logic. These are independent of Phase C's channel-map scan.

The two scans run **sequentially in `nudge::run`**: parallel-merge first (Bug 5), then corner-displacement (Bug 4). Order matters because a parallel-merge can move a corner OUT of a halo (incidental fix), eliminating work for Phase D. Order also matters because a corner-displacement could undo a parallel-merge's gain by moving a segment back to its old row; this is acceptable because the corner-displacement only fires when the merged corridor still has a corner-in-halo, which is genuinely the bug we want to fix.

**Success criteria:**
- `cargo test --workspace` adds 3 new green tests, plus `route_corners_clear_non_endpoint_node_halos` flips from ignored to green.
- `back_edges_share_return_corridor` (un-ignored in Phase C) STILL green.
- `back_edge_attach_does_not_pierce_source_perimeter` (the load-bearing exit-stub test) STILL green.
- Snapshot churn ≤ 60 cumulative across Phases C+D.

**Estimated effort:** 1 session (≤ 4 hours).

---

## Phase E — Integration + gallery + snapshot review

**Goal**: ensure the full diagram corpus renders correctly post-nudge, classify snapshot diffs as Bucket A/B/C, accept the changes, and document.

**Specific edits:**

1. **Run gallery** (`scripts/render-gallery.sh` per `feedback_render_gallery_check.md`): visually inspect `/tmp/gallery_render.txt` for all 39+ diagrams. Specifically check:
   - **Diagram 6** (state machine): `┴` exit stub still present below `Running` box; `final_state_renders_at_rightmost_column` regression test still green.
   - **Diagram 9** (Bug 4 trace fixture per CHANGELOG): the "5 errors" trace area should now have route corners pulled out of node halos.
   - **Any diagram with multiple back-edges in LR direction**: confirm corridors merge.
   - **Mindmap, gantt, sequence, sankey, pie, journey, ER, requirements, quadrant, packet, block, class, state, git-graph, timeline, xy-chart, architecture**: confirm no incidental visual change. The nudge pass should be a strict no-op on diagrams that don't have parallel back-edges or fan-in fixtures.

2. **Snapshot review** — `cargo insta test --workspace --review`. For each diff:
   - **Bucket A (Improvement)**: corridor merged, halo cleared, no other geometry changed → accept.
   - **Bucket B (Neutral)**: a route shifted by 1 cell but the visible glyph composition is equivalent (e.g. `─┐` becomes `┐─` due to corner relocation; not a regression but a stylistic change) → accept.
   - **Bucket C (Regression)**: arrow tip displaced, label collision, exit-stub pierced, junction glyph wrong → STOP, do not accept, root-cause and fix or revert per scope ceiling.

3. **Crossings test** (`cargo test --test crossings`): re-run, compare against pre-nudge baseline. Expected: ≤ +5 absolute crossings across 19 fixtures (most fixtures should improve or stay flat; the corner-displacement may add 1 crossing in tight fan-in cases).

4. **Targeted re-runs** of high-risk regression tests:
   - `cargo test back_edge_attach_does_not_pierce_source_perimeter`
   - `cargo test back_edge_source_attach_does_not_pierce_rounded_box_bottom`
   - `cargo test edge_labels_not_flush_against_thick_or_dotted_lines`
   - `cargo test diamond_interior_has_no_routing_glyphs`
   - `cargo test routes_do_not_hug_non_endpoint_node_borders` (Bug 4 regression guard added in 0.42.6)
   - `cargo test final_state_renders_at_rightmost_column`
   - `cargo test subgraph_border_does_not_overlap_downstream_node_box`
   - `cargo test subgraph_bottom_border_has_at_most_one_junction_glyph`

   All must remain green.

5. **CI gates** (per `MEMORY.md` rule 21):
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - `cargo deny check`

6. **CHANGELOG entry** in `crates/mermaid-text/CHANGELOG.md`:
   - Move "Bug 4 — Route corners in non-endpoint node halos" out of "Known limitations" into "Fixed".
   - Move "Bug 5 — Excess vertical canvas / unshared back-edge corridors" out of "Known limitations" into "Fixed".
   - Add release note: `0.43.0 — Post-routing nudging pass`.
   - Bump `Cargo.toml` version on the crate.

7. **Scope doc** — `docs/scope-nudging-pass-2026-05-05.md`: convert from "in progress" to "complete," append final snapshot tally, link the un-`#[ignore]`d acceptance tests.

8. **Update `docs/scope-launch-quality-plan-2026-05-04.md`** execution log: mark Bug 4 and Bug 5 as fixed, link this scope doc.

**Success criteria:**
- `cargo test --workspace` 0 failures, 2 newly un-`#[ignore]`d tests green, ≤ 6 new tests added.
- Snapshot count: ≤ 60 reclassified, all Bucket A or B.
- Crossings: ≤ +5 absolute total.
- CHANGELOG, scope doc, and launch-quality plan all updated.
- Gallery visual scan: no fresh artifacts.

**Estimated effort:** 1.5 sessions (≤ 6 hours). Most of this is snapshot review (Bucket A/B/C classification at 60 files × ~30 seconds each = 30 minutes, plus the time to root-cause any unexpected diffs).

**Total Phase A–E estimate**: 4.75 sessions (~19 hours of focused work). Compare with the user's 60-90 min ceiling on the planning doc itself — this is the implementation budget.

---

## The hard correctness problem

### Direction-bit subtraction at junction cells

When two paths share a junction cell (a `┼` cross or a `├ ┤ ┬ ┴` T), both paths contributed direction bits. To erase one path's bits without disturbing the other:

1. **Re-derive this path's contribution at the junction cell.** The bits this path stamped at `(c, r)` are exactly `neighbor_bit(c, r, prev) | neighbor_bit(c, r, next)` where `prev`/`next` are this path's adjacent cells. (Identical to what `draw_routed_path` did when stamping.)

2. **Subtract**: `self.directions[r][c] &= !this_path_bits`.

3. **Recompute glyph**: `recompute_cell_glyph(c, r)` reads the surviving bits and writes the correct glyph. If the cell was `┼` (all 4 bits) and we remove `LEFT|RIGHT`, the survivors are `UP|DOWN` → glyph becomes `│`. Correct.

**This works for direction bits.** The DIR_TO_CHAR LUT is monotone in subset (any subset of bits → a glyph that's the natural restriction).

### Junction cell ownership for the obstacle layer (`EdgeOccupied*`)

The obstacle classification (`Free` / `NodeBox` / `EdgeOccupiedHorizontal` / `EdgeOccupiedVertical` / `InnerArea`) is a **single value per cell, not a bitmap**. When two paths share a `┼` cell, the obstacle is whichever the first stamper set (per `draw_routed_path` lines 1646-1651: "Only upgrade Free / InnerArea cells — don't downgrade an already-classified EdgeOccupied* to the wrong axis"). The second stamper of the perpendicular axis just leaves the existing classification.

**Subtraction is impossible** without a counter. If a cell is `EdgeOccupiedHorizontal` from path A (interior, going H) and path B's V-segment crosses it, the cell stays `EdgeOccupiedHorizontal`. Erasing path A would naively make it `Free` — but path B's V-segment still crosses it, so it should become `EdgeOccupiedVertical`.

**Decision: binary flag is sufficient.** Justification:

1. **The obstacle layer is consumed only by the A\* router** (`route_edge_with_inner_cost`, line 1452+). The A\* router runs in `route_all`, which finishes BEFORE the nudge pass. After the nudge pass, no further A\* runs read the obstacle layer.

2. **`paint_fg_path` and `overdraw_path_style`** (the post-routing consumers) read the path coordinates directly, not the obstacle layer.

3. **`draw_routed_path` is called by the nudge pass during apply (re-stamping)**. At apply time, the cell is already `EdgeOccupied*` from the OLD path (not yet erased) or from a CROSSING path. Per line 1646-1651, `draw_routed_path` only upgrades `Free`/`InnerArea`; it doesn't downgrade. So a re-stamp into a cell that's already `EdgeOccupiedHorizontal` (from a crossing path) leaves the classification alone.

4. **Worst case**: a cell where path A erases is left with stale `EdgeOccupiedHorizontal` even though only path B's vertical now crosses it. This staleness is invisible: the obstacle layer isn't read after the nudge pass.

**Conclusion: leave `EdgeOccupied*` untouched during `erase_path`.** Document this with an inline comment explaining the assumption ("the obstacle layer is read-only after `route_all` returns; nudging may leave staleness in this layer but it's never observed downstream"). If a future change makes the obstacle layer post-nudge-readable (e.g. a second routing pass), the assumption breaks and we'd need either (a) a per-cell ref counter or (b) a re-derivation of the obstacle layer from the post-nudge `paths` vector. Add a `debug_assert!` in any future code that reads `obstacles` after nudging to catch this.

### Edge case: a path nudged onto a cell that another path also occupies

Phase C/D shifts the source path onto cells that may already carry another path's bits. The new path's `draw_routed_path` ORs its bits in, producing the correct junction glyph (`┼` or `┴` etc). This is symmetric to the original drawing logic — the apply step calls `draw_routed_path(new_path)` after `erase_path(old_path)`, and the bit-OR semantics handle the merge naturally.

**One subtlety**: if the apply order is wrong (e.g. erase A then erase B then draw A' then draw B'), an intermediate state can have B's bits stale at A's old cells. **Mitigation**: apply each shift atomically (`erase_path(A_old); draw_path(A_new);`) before moving to the next shift in the queue.

### Endpoint protection

The path's destination tip cell is protected (line 1657). `erase_path` MUST unprotect before clearing the glyph. The Phase B test `erase_path_handles_tip_unprotect` validates this. If a tip is shifted onto another path's interior cell, the new tip overwrites the old glyph (per `draw_routed_path:1656` it calls `set` then `protect`); this is correct because tips are the destination of an arrow.

---

## Test strategy

### Acceptance tests (the bugs we're fixing)

Both un-`#[ignore]`d at end of their respective phase. Pre-existing trap-checks in the test bodies are retained; the new strong assertions added in Phase A stay.

| Test | File:Line | Phase | Trap-check |
|---|---|---|---|
| `back_edges_share_return_corridor` | unicode.rs:3729 | C | 5 boxes rendered + ≥2 back-edge tips |
| `route_corners_clear_non_endpoint_node_halos` | unicode.rs:3884 | D | All 5 source/sink boxes rendered + ≥4 arrows-into-Z |

### Regression tests (must remain green)

Listed in priority order by sensitivity to nudge-pass changes:

1. **`back_edge_attach_does_not_pierce_source_perimeter`** (unicode.rs:3108) — the canonical "load-bearing convention" test from prior session. The B9 `┴` exit stub depends on `back_edge_path_joins[i]` pointing at the cell that's actually the back-edge's first interior cell post-routing. **The integration point in unicode.rs MUST re-derive `back_edge_path_joins` from `paths` AFTER the nudge pass.**

2. **`back_edge_source_attach_does_not_pierce_rounded_box_bottom`** (unicode.rs:3187) — symmetric for B12.

3. **`final_state_renders_at_rightmost_column`** (unicode.rs:3266) — was deferred, now fixed by the Sugiyama post-pass landed in 0.43.0-unreleased. The nudge pass operates on path data in render space, not on the layer assignment, so it should be orthogonal to B1.

4. **`routes_do_not_hug_non_endpoint_node_borders`** (regression guard added in 0.42.6) — Bug 4 sibling guard. With Phase D shipping, this should *strengthen* not break.

5. **`subgraph_border_does_not_overlap_downstream_node_box`** (Bug 1, fixed) — orthogonal to nudging.

6. **`subgraph_bottom_border_has_at_most_one_junction_glyph`** (Bug 2, fixed) — orthogonal.

7. **`edge_labels_not_flush_against_thick_or_dotted_lines`** — labels are placed AFTER the nudge pass against the modified `paths`, so this should automatically track. Run anyway as the cited Bug 4 prior failure mode.

8. **`perimeter_back_edge_label_close_to_endpoint`** (Bug 7, fixed) — labels track shifted geometry; should remain green.

9. **`diamond_interior_has_no_routing_glyphs`** (Bug 3, regression guard) — diamond interior is `NodeBox`; nudge pass never shifts INTO a NodeBox. Should be trivially green.

### Module unit tests (new, in `nudge.rs`)

7 total (4 in Phase C, 3 in Phase D). Each constructs a synthetic `Grid` + synthetic `paths`, runs the relevant scan/plan/apply, and asserts on the resulting grid state and `paths` mutation. No `render()` call — pure unit-level coverage of the nudge logic.

**Trap-check discipline (per `MEMORY.md` rule 20):** for each unit test, verify a no-op nudge implementation (`fn run(...) {}`) cannot satisfy the assertion. Concretely:

- `parallel_merge_with_no_obstacle_between_succeeds` — assert `paths[idx_a].fixed_row != paths[idx_b].fixed_row` BEFORE nudge AND `paths[idx_a].fixed_row == paths[idx_b].fixed_row` AFTER. A no-op fails the AFTER check.

- `corner_in_non_endpoint_halo_is_shifted` — assert the specific corner cell has changed coordinates. A no-op fails.

- `corner_in_endpoint_halo_is_pinned` — assert the corner cell is UNCHANGED. A no-op trivially satisfies; this test is structurally weak. **Strengthen**: also assert the path's full coordinate vector is bit-equal to the input (no spurious mutations elsewhere).

- `parallel_merge_blocked_by_node_box_aborts` — assert the path is unchanged AND assert that an obstacle-free version of the same fixture WOULD have been merged (run the merger on a parallel synthetic fixture without the blocking box, assert success there). This is the trap-check pattern: prove the assertion can fail by constructing the success case.

### Integration test (new, ≤ 1)

If snapshot review shows churn near 60 files, add a single integration test that pins the post-nudge behaviour for a representative diagram (e.g. the back-edge-share fixture from Bug 5). Otherwise skip — the snapshot suite already covers integration coverage.

### Snapshot suite

`crates/mermaid-text/tests/snapshots/` (116 files). After Phase E:
- `cargo insta test --workspace --review` walks each diff.
- Bucket A/B = accept; Bucket C = revert.
- Final accepted diff count target: ≤ 60.

### Crossings suite

`crates/mermaid-text/tests/crossings.rs` (19 fixtures). Track delta per fixture; max acceptable + 5 absolute crossings. The nudge pass typically reduces crossings (parallel-merge eliminates a row; corner-displacement may add 1 if the corner moves into a less-direct location).

---

## Risk register

10 risks, ranked by likelihood × impact:

### R1 — `back_edge_path_joins` desync (HIGH likelihood, HIGH impact)

**The risk**: `back_edge_path_joins` is computed at unicode.rs:687 BEFORE routing, recording `(col, row)` of the cell where the `┴` exit stub will be stamped. If the nudge pass shifts the back-edge's first interior cell, the stored coordinate no longer matches the path, and the `┴` stamp lands in a wrong cell (or the stamp's `current != '─'` guard at line 916 fires and the stamp is silently skipped, leaving a `─` instead of `┴`).

**Mitigation**: refactor the population of `back_edge_path_joins` to occur AFTER `nudge::run`. Concretely: at line 687 push `(edge_idx, /* recompute later */)`, then after line 707 walk the back-edges and re-derive `(col, row)` from `paths[edge_idx]`. ~5 lines of code; encoded as part of the integration step.

**Detection**: `back_edge_attach_does_not_pierce_source_perimeter` is the canary. If it fails, the desync is the root cause.

### R2 — `EdgeOccupied*` staleness exposed by future code (LOW likelihood, MEDIUM impact)

**The risk**: per the "hard correctness problem" decision, the obstacle layer is left stale after nudging. If a future change adds a second routing pass or a renderer that reads `obstacles[r][c]`, it sees stale data.

**Mitigation**: add `debug_assert!(!self.has_been_nudged || cfg.allow_stale_obstacles, ...)` guard in any future obstacle-reading code path. Document the assumption as a comment in `Grid::erase_path`. Track with a `Grid::has_been_nudged: bool` field if needed (1 byte, negligible).

**Detection**: future test failure. Currently no test exercises post-nudge obstacle reads.

### R3 — Label re-placement collision after path shift (MEDIUM likelihood, MEDIUM impact)

**The risk**: a nudged path's modified geometry can put labels into newly colliding positions. Per the existing `placed_labels` collision registry (unicode.rs:713), the label-placement code already retries up to 4 candidate positions before silently dropping. So the risk is "label silently dropped" rather than "label overlaps another label."

**Mitigation**: monitor `cargo test --workspace` for assertion failures in label-placement tests. The 4 most relevant: `edge_labels_not_flush_against_thick_or_dotted_lines`, `perimeter_back_edge_label_close_to_endpoint`, `subgraph_label_does_not_collide_with_box_border` (if exists), and any snapshot test where a back-edge label is in the comparison region.

**Detection**: snapshot diff in label rendering; phase-E gallery visual scan would catch it.

### R4 — Snapshot churn exceeds 60 (MEDIUM likelihood, MEDIUM impact)

**The risk**: the nudge pass inadvertently affects diagrams beyond the Bug 4 / Bug 5 fixture types — perhaps because the parallel-merge scan's `MAX_NUDGE_DISTANCE` is too permissive and some diagram has two parallel forward edges that get merged unexpectedly.

**Mitigation**: tune `MAX_NUDGE_DISTANCE` conservatively (start at 2, increase only if Bug 5's acceptance test fails). The `MIN_SEGMENT_LEN_FOR_NUDGE = 3` constant guards against nudging tiny stub segments.

**Detection**: snapshot-review burden. If churn > 60, classify the excess: Bucket B (neutral aesthetic shifts) at e.g. 80 is acceptable if all are Bucket B; Bucket C means revert.

### R5 — Two paths shifted onto each other's old positions (LOW likelihood, HIGH impact)

**The risk**: shift queue contains `(A: row 5 → row 6)` and `(B: row 6 → row 5)`. Naive sequential apply: erase A from row 5, draw A on row 6 (but B is still there → junction artifact). Erase B from row 6 (but A's bits now include B's contribution — subtract is wrong).

**Mitigation**: detect "swap" patterns in the shift queue; for each pair where A's destination is B's origin, apply atomically: erase both, then draw both. Or simpler: detect ANY shift queue element whose destination cells overlap with another shift queue element's source cells, and serialise so the destination shift happens first (greedy topological sort on the shift graph). This is a known dependency in compiler register allocation; the same pattern applies.

**Detection**: unit test `parallel_merge_swap_pattern` — construct two paths that would swap rows, verify the apply order is correct (or that the merger refuses to plan a swap).

### R6 — A nudge invalidates the path's topology (LOW likelihood, HIGH impact)

**The risk**: Phase C shifts a horizontal segment to a new row. The bends at each end of the segment must extend (one cell of new vertical) to reach the new row. If the extension cell is itself part of another path, OR is `NodeBox`, the resulting "shifted path" is invalid (gap or wall-piercing).

**Mitigation**: the feasibility check in Phase C step c includes the extension cells. If any extension cell fails `obstacle_at != NodeBox`, abort the shift. **This is enforced by checking ALL cells in the new path, not just the moved segment's cells.**

**Detection**: unit test `parallel_merge_blocked_by_node_box_aborts`. Plus snapshot regression in any diagram where the nudge would have piercedi.

### R7 — Endpoint-attached corners get nudged (LOW likelihood, MEDIUM impact)

**The risk**: a corner adjacent to an attach point might get classified as "in non-endpoint halo" if the adjacent halo belongs to a third node. Phase D's exemption (path[0..2] and path[len-2..len]) avoids this, but a long path's corner might happen to be in the source's halo for non-endpoint reasons.

**Mitigation**: the exemption is "if the corner cell is `path[0]`, `path[1]`, `path[len-2]`, or `path[len-1]`, do not nudge." This is a conservative rule. Tighter: "if the corner cell is in the source's halo or the destination's halo, do not nudge." Use the conservative form first; tighten only if Phase E shows missed nudges.

**Detection**: `back_edge_attach_does_not_pierce_source_perimeter` and `routes_do_not_hug_non_endpoint_node_borders` are the canaries.

### R8 — Performance regression on large diagrams (LOW likelihood, LOW impact)

**The risk**: the channel-map scan is O(n × paths_total_cells); the corner scan is O(paths_total_cells × node_count). For typical diagrams (≤ 30 nodes, ≤ 50 edges), this is sub-millisecond. For pathological diagrams (50+ nodes), could add 1-2ms.

**Mitigation**: budget the nudge pass for ≤ 5ms on the densest gallery diagram. If exceeded, profile and optimise. This is a minor risk because P99 frame draw is < 8ms target, and the launch criteria explicitly bound this.

**Detection**: `cargo bench` if a benchmark exists; otherwise time the gallery render before/after.

### R9 — `cargo deny check` regression (LOW likelihood, LOW impact)

**The risk**: a new dependency added to the `nudge` module triggers a license/audit failure.

**Mitigation**: the nudge module needs no new dependencies (it operates on `Vec<(usize, usize)>` and `Grid` only — both already in scope). Verify with `cargo tree` post-implementation.

**Detection**: `cargo deny check` per `MEMORY.md` rule 21.

### R10 — Bug 4 fix breaks the state-diagram exit-stub (HIGH likelihood, HIGH impact — the named failure mode from prior session)

**The risk**: the prior A* attempt at Bug 4 (commit 4ebaa6f reverted) failed because the halo penalty rippled into the cell that holds the `┴` exit stub. The post-routing nudge pass mitigates this BY DESIGN — the corner-displacement scan only operates on bend cells (interior, both H and V bits), not on stub cells (which have only H or only V bits because they're not at a bend). So the `┴` cell at row+1, col=center under Running's bottom border is NEVER a corner candidate.

**Mitigation**: this is structural rather than guarded. Verified by:
1. The corner-detection logic explicitly checks "both H and V bits set" — exit stubs have only one axis.
2. Phase A's failing-reproduction validation includes running `back_edge_attach_does_not_pierce_source_perimeter` to confirm the test currently passes (against the un-nudged renderer).
3. Phase E re-runs that test as the canonical guard.

**Detection**: `back_edge_attach_does_not_pierce_source_perimeter` failure → revert Phase D, document, return to defer.

---

## What NOT to do

Explicit non-goals to prevent scope creep.

1. **Do not implement an LP/VPSC solver.** libavoid uses `IncSolver` with separation constraints because pixel-level precision matters. On a 1-cell grid, greedy channel-scan with `IDEAL_SEPARATION = 1` is sufficient (research §"Open Question #4" confirms; libavoid's `idealNudgingDistance = 4` corresponds to "1 cell" in our domain). Avoid the 500+ lines of LP code.

2. **Do not refactor `route_edge`, `route_back_edge`, `route_all`, or any A\* internals.** The nudge pass operates on the OUTPUT of routing. Adding cost-model tweaks to the router has been tried twice (commits 4ebaa6f and 516206b — both reverted). The post-routing pass is the agreed strategy.

3. **Do not change `SAME_AXIS_COST`, `CROSS_AXIS_COST`, `CORNER_PENALTY`, or `inner_area_cost`.** These are tuned for the no-nudge regime. Touching them invalidates the existing snapshot baseline.

4. **Do not add `Obstacle::NearNodeBox` or any new Obstacle variant.** The corner-displacement scan reads NodeBox locations directly to compute the halo set; no new obstacle classification is needed.

5. **Do not add reference-counting to the obstacle layer.** Per "the hard correctness problem" decision, the obstacle layer is read-only after `route_all` returns; staleness is acceptable.

6. **Do not nudge segments shorter than 3 cells.** `MIN_SEGMENT_LEN_FOR_NUDGE = 3` guards stub segments. Shifting a 1-cell stub typically detaches an endpoint or pierces a wall.

7. **Do not enable the nudge pass for forward edges in Phase C.** Phase C's parallel-merge scan should only fire on back-edges (the Bug 5 use case). Forward edges have different attach-point semantics and their corridors are computed differently. Generalising to forward edges is a future-work item; check `edge_is_back_flags[idx]` in the channel-map filter.

8. **Do not introduce a new `Cargo.toml` dependency.** All required types are already in scope.

9. **Do not write `mod.rs` re-exports for `nudge`'s internal types** (`ChannelMap`, `Segment`, `Endpoint`). They stay private to the module. Only `nudge::run` is `pub(crate)`.

10. **Do not modify the `mod tests` blocks of unrelated source files.** The temptation to add a "while I'm here, fix this old test" is real; resist. Each test edit is its own scope ceiling line item.

11. **Do not delete the `#[ignore]` tests until their respective phase's acceptance criteria are met.** Premature un-ignore = CI breakage.

12. **Do not skip the gallery visual scan** (per `feedback_render_gallery_check.md`). The 13-version mindmap-trunk regression burned through 13 patch versions of green snapshot tests pinning a buggy render. The human eye is mandatory.

13. **Do not commit a half-broken nudge pass.** Per `MEMORY.md` rule 16: "a documented 'no' beats a half-broken 'yes'." If Phase C lands but Phase D's snapshot review reveals a Bucket-C regression, revert Phase D entirely (keep Phase C if it's clean) and re-defer Bug 4.

14. **Do not amend prior commits to "fix on the way."** New commits only; per `MEMORY.md` git rules.

15. **Do not skip CI gates locally.** `cargo fmt`, `cargo clippy`, `cargo test --workspace`, `cargo deny check` before every push.

---

## Open questions

Items needing experimentation or further discussion before implementation begins.

### OQ1 — Should the nudge pass run on forward edges too?

The Bug 5 fixture is two back-edges. The scope ceiling says "back-edges only" for Phase C (rule 7 in "What NOT to do"). But the corner-displacement scan in Phase D is naturally agnostic: a forward edge whose corner lands in a non-endpoint halo deserves the same treatment.

**Answer needed before Phase D**: does Phase D fire on forward edges? Tentative answer: yes, but only when the forward edge's endpoint exemption (path[0..2], path[len-2..len]) would trigger no false-positives. Cross-check on Bug 4's fixture (`A → Z, B → Z, C → Z, D → Z` — all forward edges).

### OQ2 — Should the parallel-merge scan also handle perpendicular adjacency?

Two paths that run perpendicular through the same channel (one horizontal, one vertical) cross at one cell but don't "share a corridor." Parallel-merge as planned operates on co-axial segments. Perpendicular handling is OUT of scope.

**Confirmation needed**: re-read the Bug 5 fixture to confirm both back-edges are co-axial (both horizontal in LR layout). If so, perpendicular handling is genuinely unneeded; close this OQ pre-implementation.

Reading the fixture (`graph LR; A--B--C--D--E; E-->|back1|A; D-->|back2|B`) — both back-edges route from right to left along the perimeter row below the chain. Both are horizontal segments on different rows. **Co-axial confirmed; OQ2 closed: perpendicular handling is out of scope.**

### OQ3 — How do we encode "corridor sharing" in the snapshot suite?

The Bug 5 acceptance test asserts `height <= 9` (current 11). The snapshot for a similar fixture (`crossings__crossings_dense_multiple_back_edges.snap`) is one of the existing 116 snapshots; post-nudge its height drops by ~2. This is an Improvement (Bucket A). Does it count toward the 60-file ceiling? **Yes** — even Bucket A diffs count toward the churn budget because they require human snapshot review.

**Mitigation**: the 60-file ceiling is generous specifically because Bucket A diffs are common in this work.

### OQ4 — What if Phase C's apply step finds the destination row is partially occupied by a third path?

Channel-map detects A on row 5, B on row 6 (parallel, comparable). Plan: shift A to row 6. But row 6 has another path C running in a different column range that doesn't overlap with B's range. The shift puts A on row 6 in cells that overlap C's range — could collide.

**Mitigation**: the feasibility check checks EVERY cell of the proposed new path, not just the segment-overlap region. If a third path's cell would be `EdgeOccupiedHorizontal` and the new path's cell is also horizontal, the cells become a same-axis overlap. Currently the channel-map scan's feasibility check accepts this (it's a `SAME_AXIS` overlap, just like B's, not a NodeBox). **Pre-implementation question**: should we also reject same-axis overlaps with non-target paths? Tentative answer: **no, accept**, because the OR-bit semantics of `draw_routed_path` produce a clean shared corridor regardless of which paths share. The shared `─` is unambiguous. If a Bucket-C regression appears in Phase E from this case, revisit.

### OQ5 — Does Phase D need a per-fixture maximum nudge count?

A diamond-fan fixture might have 10+ corner-in-halo cases, each fixable. Should we cap the count to limit ripple? Tentative answer: **no cap**. The Bug 4 fixture has 3 corners (A, C, D's right-halo cells around B); fixing all 3 is the bug being defined.

**Confirmation needed**: walk Bug 4's fixture render and count corners-in-halo. Adjust if pathological.

### OQ6 — Should the nudge pass be feature-gated for first-release?

Adding `const NUDGE_PASS_ENABLED: bool = true;` and gating the call site allows fast revert without a code change. Tentative answer: **yes during development; remove for the 0.43.0 release**. The constant is a development scaffold, not production toggle.

### OQ7 — How does the integration interact with the `with_color` rendering path?

`grid.paint_fg_path(path, stroke)` (unicode.rs:795) reads the path coordinates and paints colours. After the nudge pass, the path coordinates have changed; the paint call uses the NEW coordinates. Stroke colour tracks geometry naturally.

**No action needed.** Confirmed by reading unicode.rs:795 — the call takes `&path` (post-nudge), so colours follow the new geometry.

### OQ8 — Does Phase E's gallery-render acceptance criterion need a baseline?

The `scripts/render-gallery.sh` produces `/tmp/gallery_render.txt`. To detect regressions, we need a pre-nudge baseline. Tentative answer: **yes, capture before-and-after**. Run the script before starting Phase A (capture `/tmp/gallery_baseline.txt`), then again at end of Phase E. Diff the two; expected diff is bounded to specific diagrams (the Bug 4 / Bug 5 fixtures and any test diagrams that happen to match the Bug 4/5 patterns).

---

### Critical Files for Implementation

The 5 files most critical for landing this plan:

- /Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/crates/mermaid-text/src/layout/grid.rs (add `erase_path`, share helpers with `draw_routed_path`)
- /Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/crates/mermaid-text/src/layout/nudge.rs (NEW — the nudging pass module)
- /Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/crates/mermaid-text/src/layout/mod.rs (add `pub(crate) mod nudge;`)
- /Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/crates/mermaid-text/src/render/unicode.rs (insert nudge call at line 707; refactor `back_edge_path_joins` to recompute post-nudge)
- /Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/crates/mermaid-text/CHANGELOG.md (move Bug 4 + Bug 5 from "Known limitations" to "Fixed"; add 0.43.0 release note)

---

## Summary of design decisions and judgment calls

The plan I designed lands a single `crates/mermaid-text/src/layout/nudge.rs` module that runs at one insertion point (`unicode.rs:707`, immediately after `route_all` returns) and handles both bugs through a shared `apply_shifts` infrastructure but two independent scans (parallel-merge for Bug 5 in Phase C, corner-displacement for Bug 4 in Phase D). The scope ceiling is ≤ 450 net lines, ≤ 60 reclassified snapshots, 0 regressions.

**Three judgment calls that needed resolution:**

1. **Binary `EdgeOccupied*` vs ref-counted obstacle layer.** Decided binary is sufficient because the obstacle layer is read-only after `route_all` returns; staleness is invisible to the renderer. Documented assumption with a future-proof `debug_assert!` guard recipe so a future second-routing-pass change doesn't silently break.

2. **`back_edge_path_joins` desync** (Risk R1). The integration step requires recomputing this vector AFTER the nudge pass — this is the load-bearing change in `unicode.rs` that prevents the `┴` exit-stub regression that broke prior A*-based attempts at Bug 4. Identified during architecture review of the integration point.

3. **Ordering of Phase C before Phase D.** Parallel-merge runs first because it can incidentally move corners out of halos, eliminating Phase D work. Phase D operates on the post-merge geometry, which is a more honest representation of the bug.

The plan respects all referenced project standards: scope-ceiling discipline (`MEMORY.md` rule 16), failing-reproduction-test-first with trap-checks (rules 19, 20), CI gates locally (rule 21), gallery-render visual scan (`feedback_render_gallery_check.md`), and deferral discipline (`feedback_deferral_discipline.md`) for the abort path.

**Plan path**: the document was intended to land at `docs/plan-nudging-pass-2026-05-05.md`; the read-only planning environment prevented file creation, so the full ~1,100-line plan is in this response. The parent agent can persist it verbatim if desired.
