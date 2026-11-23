# PR #62 review — ascii-dag 0.11.0 — 2026-11-23
## Decision: do not merge
Critical compatibility finding: proposed head afbb76e fails `cargo check -p mermaid-text --lib` at `crates/mermaid-text/src/layout/sugiyama.rs:740` (removed add_node_with_size) and line 809 (crate-root NodeKind now denotes scene nodes, not IR nodes).
The documented migration uses Graph::add_node with CustomNode and ascii_dag::ir::NodeKind. That small migration compiles, but `cargo test -p mermaid-text --no-fail-fast` fails 64 tests across the library, crossings, regression corpus and snapshots. A second experiment setting both dependency spacing values to 3 still fails 60 tests. Failures include architecture spatial-routing/compactness assertions, not merely snapshot drift.
Scope ceiling: 200 compatibility-code lines, zero snapshot reclassifications, zero regressions. Both attempts were reverted; no expected output was accepted. A layout-policy migration is outside this dependency-review scope and requires deliberate visual review. Keep the existing PR open; no follow-up issue created.
Reproduction patch: docs/reviews/pr62-migration-attempt.patch applies to afbb76e. Logs: /tmp/markdown-pr-validation/deps62-{check,migration,legacy-spacing}.log. Published API changes: https://github.com/AshutoshMahala/ascii-dag/releases/tag/v0.11.0 and the crate's CHANGELOG.md.
## Red-Team Checklist
- Checked data safety: no persistence/schema change, but node geometry and spacing contracts differ.
- Checked concurrency: synchronous layout path; no new application task or retry mechanism.
- Checked degradation: original PR cannot build; migrated version breaks existing layout assertions.
- Checked security/config: no added application input, credentials or network path; default feature/API changes reviewed.
- Checked performance: new layout internals cannot be approved solely on upstream benchmarks when behavioral gates fail.
- Checked tests: existing exact-output and geometry regressions reproduce failure without weakening assertions. No manual terminal claim.
