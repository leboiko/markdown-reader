# PR #43 review — self-loop and back-edge routing

Reviewed: 2026-08-29  
PR: https://github.com/leboiko/markdown-reader/pull/43  
Issue: https://github.com/leboiko/markdown-reader/issues/42

## Result

Two actionable correctness findings. The dependency, security, attribution,
versioning, and release-metadata pass found no actionable issues.

## Findings

### High — reverse-direction self-loops corrupt rectangle borders

`crates/mermaid-text/src/render/unicode.rs:1517` applies the new normal-exit /
back-entry self-loop pair to every flow direction. In `RL` and `BT`, a single
node starts at column or row zero; `saturating_sub(1)` therefore places the
source attachment on the node border. Exact connector stamping subsequently
replaces that border cell.

Observed output includes a missing left border for `flowchart RL` (`─ Retry │`)
and a split top border for `flowchart BT` (`┌───┌───┐`). ASCII output inherits
the same corruption. The direction-agnostic support claim in
`crates/mermaid-text/README.md` makes this user-facing.

Recommended fix: either reserve real left/top routing margin before using
normal exits, or keep issue #42's self-loop repair explicitly limited to its
reported `LR` envelope until reverse-direction margins exist. Add Unicode and
ASCII regressions proving `RL` and `BT` retain intact rectangle borders.

### Medium — self-loop test does not prove the source route joins the box

`crates/mermaid-text/tests/edge_topology.rs:50` computes reciprocal direction
information but only asserts that a connector arm points at a non-whitespace
cell. A route arm aimed at a plain `│` border therefore passes even though the
border has no right-facing arm. The self-loop tests also do not directly pin
the `Retry` row's `├` source tee.

Recommended fix: assert the exact source tee beside `Retry` and a nonblank
routed neighbor, matching the stronger per-source checks already used by the
multi-back-edge fixture. Run the strengthened assertion against the unfixed
implementation first and confirm it fails when the source stamp is absent.

## Verification reviewed

- `cargo fmt --all -- --check` — pass
- `cargo clippy --all-targets -- -D warnings` — pass
- `cargo test --workspace` — pass
- `cargo doc --no-deps --workspace` — pass with existing link warnings
- `cargo deny check` — pass after `h2` 0.4.19 advisory update
- `cargo check --workspace --locked` — pass
- Existing renderer snapshots — 123 passed, zero reclassified

## Specialist passes

- Rust correctness/layout: two findings above.
- Security/dependencies/OSS metadata: no actionable findings; versions and
  lockfile are consistent, `h2` is dev-only, MIT metadata and @jserv credit are
  intact, and no secrets, unsafe code, or new external input path were added.
- Test reliability: local audit confirmed the six issue edges are counted and
  ASCII is pinned to Unicode conversion; the source-connectivity weakness is
  captured above.
