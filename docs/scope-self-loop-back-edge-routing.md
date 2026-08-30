# Scope: self-loop and back-edge routing (#42)

Status: focused implementation complete; validation and review in progress.

## Observable failures

- A one-node LR self-loop collapses to a one-cell vertical stub, so the edge
  never visibly leaves and returns to distinct ports on the node.
- Three TD back edges targeting the first node lose their arrow tips, while
  source-side junctions terminate at node borders instead of joining the
  routed perimeter paths.
- Unicode and ASCII rendering exhibit the same structural failures.

The regression tests in `crates/mermaid-text/tests/edge_topology.rs` pin the
actual endpoint counts and route geometry. They require every reported edge to
remain visible, so deleting or skipping an edge cannot satisfy them.

## Hard scope ceiling

- At most 250 changed production-code lines.
- At most four existing snapshots reclassified.
- Zero existing regressions tolerated by the current workspace test harness.
- No layout-engine rewrite or public API change.

If a complete fix exceeds this envelope, revert the attempt and record the
failed approach here instead of shipping a partial fix.

## Rejected broad attempt

The first implementation generalized endpoint spreading and connector repair
to every back edge and self-loop shape. Although the issue fixtures passed, it
reclassified 21 existing snapshots and changed established rounded-state-node
output. That exceeded the snapshot ceiling, so the generated snapshot changes
and generalized behavior were reverted.

The retained fix is limited to the issue's rectangle flowchart envelope. It
uses distinct self-loop ports, spreads rectangle back-edge fan-in along the
actual attachment side, preserves routed endpoints during corridor nudging,
and reconstructs shared source junctions from the final path directions.
Existing rounded state-diagram rendering remains byte-identical.

## Validation gate

- Focused Unicode and ASCII endpoint-topology regressions.
- Existing `mermaid-text` tests and regression corpus.
- Workspace formatting, Clippy, tests, docs, dependency policy, and Nix checks
  matching repository CI where locally available.

The current Rust 1.98 Clippy also surfaced two pre-existing lints in unrelated
viewer code: `drain_collect` in `src/markdown/renderer.rs` and
`chunks_exact_to_as_chunks` in `src/mermaid.rs`. Both are behavior-preserving
one-line mechanical updates required for the repository's warnings-denied CI;
they add no issue-specific behavior.

`cargo deny check` found RUSTSEC-2026-0258 in the existing dev-test `h2`
0.4.13 dependency. The lockfile was updated to the latest compatible patched
0.4.19 release; this does not enter the renderer's runtime dependency graph.

`cargo doc --no-deps --workspace` completes with the repository's existing
intra-doc-link warnings. `cargo-audit` and Nix are not installed in this local
environment; `cargo deny check` covers the RustSec advisory gate and passes.
