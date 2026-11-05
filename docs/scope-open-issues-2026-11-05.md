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
- Open PRs #56, #49 and dependency updates do not implement #54 or #55;
  preserve their scope and authorship; do not absorb unrelated changes.
- Worktree starts at origin/master 83bc806; original checkout has unrelated
  untracked writing which is left untouched.

## Verification

Pending reproduction tests, full local gates, review, package dry runs, and
release artifact verification. Intended versions: mermaid-text 0.58.0 and
markdown-tui-explorer 1.35.4, after checking the package registry.

## Reproduction evidence

- #55 baseline panics at flowchart.rs:958: byte range starts at 2 but ends at 1.
- #54 baseline row 5, column 20 is `[lifeline, space]`, expected two bar cells.
  The test derives the lifeline from participant box edges and treats trimmed
  trailing cells as spaces.
- A renderer-only expansion cannot distinguish inline activation or notes before
  an explicit directive. Preserve the explicit directive's parsed note index in
  `Activation::start_note`; adding a public field requires minor version 0.58.0.
