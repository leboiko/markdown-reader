# Open issue fixes and release — 2026-11-05

## Scope ceiling

At most 150 changed production Rust lines; zero existing snapshots reclassified;
zero regressions tolerated. Use the existing workspace test harness. Failed
approaches must be reverted and recorded here before a different approach.

## Intake

- #55: flowchart single slash/backslash labels panic. No linked open PR.
- #54: explicit activation starts after intervening note rows. No linked open PR.
- #48: existing raster/tag and protocol mitigation merged in #53. Real WSL/WezTerm
  display is unavailable on this Mac. User authorized releasing verified fixes
  while keeping #48 open.
- Open PRs #56 and #49 do not implement #54 or #55 and remain outside scope.
  Dependency PR #45 is incorporated below when the advisory gate fails.
- Worktree starts at origin/master 83bc806; original checkout has unrelated
  untracked writing which is left untouched.

## Verification

Reproductions, stable/MSRV suites and local gates passed; review, clean package
dry runs and release artifact verification follow. Intended versions: mermaid-text 0.58.0 and
markdown-tui-explorer 1.35.4, after checking the package registry.

## Reproduction evidence

- #55 baseline panics at flowchart.rs:958: byte range starts at 2 but ends at 1.
- #54 baseline row 5, column 20 is `[lifeline, space]`, expected two bar cells.
  The test derives the lifeline from participant box edges and treats trimmed
  trailing cells as spaces.
- A renderer-only expansion cannot distinguish inline activation or notes before
  an explicit directive. Preserve the explicit directive's parsed note index in
  `Activation::start_note`; adding a public field requires minor version 0.58.0.

## Release security gate

The first full workspace test run passed. cargo-deny and cargo-audit failed on
new pulldown-cmark, Rustls, webpki and Tokio advisories. Reviewed #45 and posted
its remaining Tokio advisory on GitHub before integrating it. Merged the original
Dependabot commit e5c8421 (not squashed/reimplemented); refreshed five affected
packages to current compatible patched releases. Four Cargo.lock conflicts were
Windows dependency references; resolved them to the incoming references and let
Cargo resolve the final graph. No advisory exceptions added.

## Final local verification

- Stable Rust 1.99.1: 1,674 passed, 0 failed, 3 existing ignored tests.
- Rust 1.92 MSRV: full workspace including doctests passed.
- rustfmt; workspace/all-target/all-feature Clippy with warnings denied: pass.
- cargo doc: pass, existing intra-doc-link warnings remain outside this release.
- cargo deny: advisories, bans, licenses and sources all pass, no new exceptions.
- cargo audit: pass, five existing allowed transitive warnings.
- Debug workspace build and application version/help smoke checks: pass.
- CLI issue #54 output inspected in Unicode and ASCII; contiguous bar cells.
- 64 production Rust lines added/removed, below 150-line ceiling; zero snapshots
  changed. New tests include negative ordering and nested multiline-note cases.
- Nix is not installed locally; platform-specific Nix builds require CI.
- Initial package dry run correctly refused uncommitted version/README changes;
  commit these then rerun from a clean tree before publishing.
