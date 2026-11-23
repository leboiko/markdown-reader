# PR #65 review — httpmock 0.9.0 — 2026-11-23
## Result
No unresolved code finding in the validated combined candidate. Development dependency only; repository search finds no direct httpmock API usage. Removed matchers/state/body helpers and HTTP/2 default feature do not affect this app. Removes obsolete async pool/event-listener dependencies; full test suite passes.
Original Dependabot commits are preserved by merge commits. Eight compatible PRs (#46, #47, #60, #61, #63, #64, #65, #67) are validated together and submitted through #67; #62 is excluded (see PR-62-review.md).
## Validation and architecture
No production source changes required. Strengthened render_four_target_diagrams from allowing two failures to requiring all four fixtures to render and rasterize; all four pass. Full workspace tests pass on stable and Rust 1.92; workspace/all-target Clippy, formatting, docs, cargo-deny and cargo-audit checked. Existing audit policy reports five nonblocking warnings; existing doc warnings remain. Nix is unavailable locally, so remote Nix builds and binary smoke tests must pass before merge. No manual terminal visual claim.
Evidence: https://github.com/httpmock/httpmock/releases/tag/v0.9.0; proposed diff and package graph; affected application call sites; /tmp/markdown-pr-validation/deps-*.log. Recommend merge only after combined remote checks pass.
## Red-Team Checklist
- Checked data safety: config/session serde and directory APIs, retained patched lock entries; no schema/migration or application write-path change.
- Checked concurrency: existing rendering worker cap and cached font initialization retained; no new application task/retry loop. Dev HTTP pool removal does not affect production.
- Checked degradation: existing parse/render fallback, empty/malformed input and runtime integration tests retained; Nix startup verified by remote smoke gate.
- Checked security/config: registry checksums and advisory/license/source/banned-crate gates; action inputs and workflow permissions unchanged; no secrets introduced.
- Checked performance: no application hot-path probe/allocation added; renderer changes guarded by existing corpus and timing tests, no benchmark claim.
- Checked tests: full workspace, MSRV and exact-output tests run; zero snapshots reclassified. #62's failing experiment is excluded, not hidden by expected-output changes.
