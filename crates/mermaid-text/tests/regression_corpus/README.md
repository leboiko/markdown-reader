# Regression Corpus — Routing Safety Net

This directory is the **frozen-gallery snapshot harness** for `mermaid-text`.
It exists to give the team a reliable safety net before any change touches the
routing-attach code path (where bugs B3, B9, and B12 live).

## What is captured

| Category | Count |
|----------|-------|
| Flowcharts (graph / flowchart) | 29 sources |
| State diagrams (stateDiagram-v2) | 11 sources |
| Entity-relationship diagrams (erDiagram) | 3 sources |
| Gantt diagrams | 2 sources |
| Timeline diagrams | 2 sources |
| Git graph diagrams | 2 sources |
| **Total** | **51 sources, 102 snapshots** |

Each source is rendered **twice**:

- `<name>.snap` — natural-size render (`render(src)`, no width constraint)
- `<name>.width80.snap` — 80-column-constrained render
  (`render_with_width(src, Some(80))`). Bug B3 only manifests under width
  constraints, so this variant is essential.

Snapshots are stored under `tests/regression_corpus/snapshots/`, isolated from
the existing `tests/snapshots/` directory used by the per-feature unit tests.

## Running the harness

```shell
# Must be green before touching any routing code
cargo test --test regression_corpus
```

## Updating snapshots after an intentional change

```shell
INSTA_UPDATE=always cargo test --test regression_corpus
cargo insta review
```

Work through each diff in `cargo insta review`. For each changed snapshot,
classify it manually as one of:

| Class | Meaning |
|-------|---------|
| **A — Improvement** | The new output visibly resolves a known issue (e.g. `├` no longer bleeds into a node border row, a box corner is now intact). Accept. |
| **B — Neutral** | Visual reflow with the same semantic content — nodes and edges all present, just repositioned. Accept with a comment. |
| **C — Regression** | Node names missing, borders broken, edges crossing where they did not before, or the output is visibly worse. Reject the change. |

**If any snapshot diff is class C, revert the routing change.** The fix is not
ready.

## Workflow for fixing routing bugs (B3, B9, B12)

1. Confirm baseline is green:
   ```shell
   cargo test --test regression_corpus
   ```
2. Make your change to the routing code.
3. Run the corpus again:
   ```shell
   cargo test --test regression_corpus
   ```
4. Review diffs:
   ```shell
   cargo insta review
   ```
5. Classify each diff (A/B/C — see table above). Accept only A and B diffs.
6. If all diffs are A or B, run the full suite and open a PR:
   ```shell
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   ```

## Sources of particular interest for B3/B9/B12

These diagrams are the highest-priority regression targets:

- `flowchart_app_db_architecture` — the canonical App→PostgreSQL architecture
  chart. B3 manifests here under 80-column width constraint.
- `state_circuit_breaker` — the primary v1 acceptance test. B9 (back-edge
  source-attach) is most visible here.
- `flowchart_back_edge_lr` and `flowchart_back_edge_td_cycle` — exercises the
  back-edge perimeter routing that B12 affects.
- `state_fork_join_full` — choice/fork/join composite with multiple back-edges.
- `state_composite_keyboard_lock` and `state_nested_composites` — deep
  composite nesting where deferred-attach ordering matters.

## Known baselines that include existing bugs

The following snapshots were captured **with the current (pre-fix) renderer**.
Their output is the regression baseline; it may look wrong. That is intentional
— the snapshot documents the current behaviour so Phase 3 improvements are
visible as class-A diffs:

- `state_circuit_breaker.snap` / `state_circuit_breaker.width80.snap` — the
  HALF_OPEN back-edge routing may show routing artefacts (B9).
- `flowchart_back_edge_lr.snap` — back-edge anchor may not be perfectly
  attached to the source lifeline (B12).

These will improve to class-A diffs when Phase 3 ships the actual fixes.
