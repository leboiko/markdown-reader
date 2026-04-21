# Scope — ROADMAP item #6: layered-layout improvements

**Status:** **DECIDED 2026-04-22.** Strategy A (incremental
improvements to our `layered.rs`), with A.3 (median + transpose
passes) in scope. No new runtime deps. Three ship cycles planned:
0.10.0 (A.1: dummy nodes), 0.10.1 (A.2: Brandes-Koepf), 0.10.2
(A.3: median + transpose).

This document remains in the repo as the historical record of how
we evaluated the alternatives. The full decision matrix and reasoning
are below.

## TL;DR

Three viable strategies, in increasing order of code-touch:

| | Strategy | Risk | Effort | When to pick |
|---|---|---|---|---|
| **A** | Incremental: add dummy nodes + median + transpose passes to our existing `layered.rs` | Low | ~2 days | We want to ship soon and own the code |
| **B** | Adopt `ascii-dag` 0.9.1 (terminal-native, has subgraph support) | Medium | ~3 days | We want the algorithm work outsourced and an actively-maintained dep |
| **C** | Adopt `rust-sugiyama` 0.4.0 | **High — open hang bug** | ~3 days | Only if we first patch the upstream `from_graph` infinite loop ourselves |

**Recommendation: Strategy A** unless we hit a wall. Three reasons:
(1) our `layered.rs` already does barycenter crossing minimisation
with iterative sweeps and best-seen retention — the gap to "real
sugiyama" is smaller than the ROADMAP entry implied; (2) `rust-sugiyama`
has an unresolved open issue (#25, since Jan 2026) that causes
`from_graph` to hang on certain inputs — we cannot ship a hang;
(3) `ascii-dag` looks compelling but its coord-egress API for our
use case isn't audited yet.

If A doesn't deliver enough, B is the next step. C is off the table
until the upstream hang is fixed.

---

## Current state — what `layered.rs` actually does today

The Explore agent found the existing layered layout is more
sophisticated than the ROADMAP entry implied:

- **Layer assignment**: longest-path from sources, with bounded
  iteration for cycles, plus special handling for orthogonal-direction
  subgraphs (collapse + re-propagate downstream). 100 LoC.
- **Within-layer ordering**: barycenter heuristic with iterative
  forward/backward sweeps (max 8 passes, early exit at 4 non-improving),
  full crossing-count evaluation, **best-seen retention**. ~200 LoC.
  This is most of what real sugiyama does for crossing min — we just
  don't have the median or transpose variants on top of barycenter.
- **Position computation**: `(layer, rank) → (col, row)` grid coords,
  direction-aware (TD/BT/LR/RL), inter-layer gaps that auto-widen
  for label fit, subgraph boundary widening. ~150 LoC.
- **Edge routing**: A* on a character grid, `EDGE_SOFT_COST = 4.0`
  to push parallel edges into separate channels. Independent of layout.

**What we DON'T have** that real sugiyama provides:
1. **Long-edge dummy nodes** — edges spanning >1 layer take detours
   in the A* router instead of having reserved straight channels.
   This is the main cause of the dependency-graph spaghetti.
2. **Median + transpose crossing-min passes** — barycenter alone
   misses some optimisations on dense graphs.
3. **Brandes-Koepf coordinate assignment** — we use a simpler
   evenly-spaced layout. Brandes-Koepf produces more compact,
   prettier results for typical cases.
4. **Cycle removal via greedy feedback arc set** — we handle cycles
   via iteration cap; back-edges then need perimeter routing.

**Critical insight**: items (1) and (3) are the biggest wins. (2) is
incremental. Adopting a sugiyama library buys all four; doing them
incrementally buys (1) and (3) at lower cost and zero new deps.

---

## Strategy A — Incremental improvements to `layered.rs`

**The pitch**: our barycenter pass already gets us most of the way.
Add (1) dummy nodes for long edges and (3) Brandes-Koepf coord
assignment in two focused PRs. Keep (2) median pass as optional
follow-up.

### Phases
- **A.1 — Dummy nodes for long edges** (~6h). When an edge spans >1
  layer, insert a dummy node per intermediate layer. Update the
  router to recognise dummy nodes as straight-through cells. Result:
  long edges become reserved channels, dependency-graph cleans up.
- **A.2 — Brandes-Koepf coordinate assignment** (~8h). Replace the
  evenly-spaced layout with Brandes-Koepf's compact placement.
  Pretty wins on most diagrams.
- **A.3 (optional) — Median + transpose crossing passes** (~5h).
  Add as additional iterations of the barycenter loop. Marginal win
  on already-good cases; bigger win on pathological dense graphs.

### Trade-offs
- ✅ **Zero new deps**, no upstream-bug risk, full code ownership.
- ✅ **Snapshot regression risk is minimal** — incremental changes
  produce incremental visual deltas, easier to triage.
- ✅ **Code quality**: stays in our codebase under our naming and
  abstractions (the user's "code as art" rule).
- ⚠️ Median + transpose crossing-min implementations are non-trivial
  to write from scratch (~150 LoC each, well-studied algorithms).
- ⚠️ Brandes-Koepf is genuinely tricky — original paper is 18 pages.
  Reference implementation in `rust-sugiyama` (~300 LoC) is readable
  enough to port.

### When to pick A
The user's "code is art" rule and OSS-quality preferences favour
A. We get full control, no dependency risk, and the algorithms are
well-documented enough to implement cleanly.

---

## Strategy B — Adopt `ascii-dag` 0.9.1

**The pitch**: `ascii-dag` is **terminal-native** (positions snap to
character cells), supports subgraph clustering natively, dual MIT/
Apache-2.0 license, more active than `rust-sugiyama` (18 releases),
zero deps, `no_std`. Median + barycenter crossing reduction built-in.

### Risks / unknowns
- 🔴 **API audit needed** (~2h spike): does `ascii-dag` expose raw
  per-node coordinates we can feed to our existing A* router? Its
  API is "optimised for rendering" per the search agent, so it may
  bundle layout + render together in ways we'd need to unpick.
- 🟡 If audit clears, this becomes the strongest pick — terminal-
  native means no float-to-cell translation, and built-in subgraph
  support means we don't have to invent the cluster-constraint logic.

### Effort
- 2h spike (verify coord egress) + ~3 days integration if it clears.

### When to pick B
If A's algorithm work feels heavier than expected, or if we want
to eventually offload more of the layout pipeline.

---

## Strategy C — Adopt `rust-sugiyama` 0.4.0 — **NOT RECOMMENDED**

**The pitch the prior-art research suggested**: full sugiyama pipeline
in one dep — layer assignment, crossing min, Brandes-Koepf,
auto-cycle handling.

### Why we should NOT do this right now

🔴 **Open Issue #25 (Jan 2026, unresolved)**: `from_graph` enters
an infinite loop on certain inputs. Trigger condition unknown from
public info. We cannot ship a renderer that may hang.

Other concerns:
- 🟡 Intermittent maintenance (single maintainer, multi-month gaps).
- 🟡 No native subgraph support (we'd implement that ourselves).
- 🟡 26 stars, 11k total downloads — small adoption signal.

### Path to making C viable
Reproduce Issue #25 ourselves, submit a PR upstream with a fix, wait
for merge + release. ~1 day of unplanned upstream work plus
indeterminate wait. Not worth it given Strategy A is available.

---

## Decision matrix

| Concern | A: incremental | B: ascii-dag | C: rust-sugiyama |
|---|---|---|---|
| Time to ship | 2 days | 3 days + 2h spike | 3 days + upstream fix |
| New runtime deps | 0 | 1 (zero-dep crate) | 2 (sugiyama + petgraph) |
| Hang-bug risk | none | low | **HIGH (open #25)** |
| Subgraph handling | reuse existing | native in dep | invent ourselves |
| Long-term maintenance | own everything | external dep, active | external dep, intermittent |
| Code-as-art alignment | ★★★ | ★★ | ★ |

---

## Strong recommendation

**Pick A.** Ship in 3 phases as 0.10.0 (dummy nodes), 0.10.1
(Brandes-Koepf), 0.10.2 (optional median/transpose). Each phase has
focused snapshot impact and clean rollback.

If during A.1 we discover the dummy-node + router work is messier
than expected, fall back to B (`ascii-dag` audit + integration).

Keep C as a tracked option in case `rust-sugiyama` ships a fix for
#25 and gains traction.

---

## Open questions for the user

1. **A vs B vs C** — go with the recommendation (A) or pick differently?
2. **Phase A.3 (median + transpose)** — ship in 0.10.x or defer
   until we see whether A.1 + A.2 alone close the gallery-quality gap?
3. **Snapshot-review tolerance** — for Phase A.1, ~30 flowchart/state
   snapshots will change. Accept all improvements + neutrals, hold
   the release on >2 regressions?
4. **Dependency budget** — strict zero-new-deps preference (favours A)
   or open to a well-vetted dep (B)?

Once we pick a strategy I'll create the per-phase task list and
start building.
