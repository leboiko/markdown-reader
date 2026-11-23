# PR #56 review — 2026-11-23
Reviewed 4f0f635, preserving contributor commits and credit. Critical finding at src/app/file_ops.rs:249: completion wrote into the currently active tab, losing another document's view. Original failing test changed /fake/third.md to /fake/other.md; out-of-order completion also changed latest.md to old.md.
Fix: per-tab pending replacement path retains original content until success, routes completions to the requesting tab, ignores superseded/closed targets and avoids background focus theft. Failure clears the reservation, allowing retry. Existing public event payloads remain compatible.
Scope/failed attempt: first correction returned before processing pending source jumps on an already populated tab (cursor 0 != 2). Removed that premature return condition; existing regression unchanged. No snapshot reclassification or tolerated test regression; combined additional-change ceiling 250 lines.
Validation: workspace tests on stable/Rust 1.92, workspace/all-target Clippy, rustfmt, docs, cargo-deny and cargo-audit; logs /tmp/markdown-pr-validation/pr56-*.log. New tests cover wrong-tab delivery, reversed completion, closed targets, failure/retry, empty files and focus retention. Original contributor regression retained.
## Red-Team Checklist
- Checked data safety: no filesystem writes/migrations; failed replacement retains original path/content. Path-based ownership follows existing event model; same-path overlapping reads retain existing snapshot semantics.
- Checked concurrency: switched active tab, superseded selection and closed target; reservations are owned by the tab and removed with it.
- Checked degradation: read failure clears reservation; retry and empty content tested. No new external service or retry loop.
- Checked security/config: existing path read only; no shell, secrets or network introduced. Dependency-policy changes inherited from reviewed #49.
- Checked performance: at most 32 tabs scanned, one pending path per tab, no unbounded queue added.
- Checked tests/architecture: existing app action harness drives real completion logic and exact content/path assertions; full workspace catches source-jump compatibility. No manual terminal exercise.
Recommendation: merge after gates pass. Remaining policy/doc warnings are recorded in PR-49-review.md; no unresolved merge-blocking code finding.
