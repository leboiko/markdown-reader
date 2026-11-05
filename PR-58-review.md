# PR #58 review

## Executive summary

Reviewed the complete PR diff and affected parser, sequence model, renderer,
package manifests, lock graph, release workflows and tests. No actionable
findings remain. Two source defects are fixed with baseline-failing regressions.
The integration retains Dependabot's #45 commit and adds current advisory fixes.

## Findings and recommendations

No remaining Critical, Major or Minor findings in scope. Preserve the 0.58 minor
bump because adding `Activation::start_note` changes public struct construction.
Publish the library and verify crates.io visibility before the parent package.

## Testing assessment

1,674 tests pass on stable and the complete suite also passes Rust 1.92. Existing
snapshot files are unchanged. Cell-by-cell bar tests cover explicit/inline
activation ordering, nested lanes, multiple/multiline notes and post-deactivation
lifelines in Unicode/ASCII. Parser tests assert exact labels, shapes, node counts,
edge endpoints and rendered labels; empty/Unicode slanted shapes stay supported.
The packaged library dry run compiles independently. fmt, Clippy, docs, deny,
audit and binary/CLI smoke checks pass. There are three existing ignored tests,
existing doc-link warnings and five existing audit warnings. No local Nix binary.

## Architecture assessment

The parser records explicit directive ordering without changing message indices;
the renderer translates only matching note positions into earlier lane rows.
Inline/public constructed spans keep message-based timing with `None`. Bounds
checks and saturating arithmetic preserve malformed-public-model behavior.
The flowchart guard only rejects overlapping same-slash delimiters, leaving
four-byte empty shapes and mixed slanted shapes intact.

## Red-Team Checklist

- [x] Data safety/model compatibility: no persistence/schema changes. Public
  field addition gets minor 0.x version and migration guidance in README/changelog.
- [x] Concurrency/idempotency: changed parser/renderer paths are synchronous and
  local; no new background work, caches, retries or shared mutable state.
- [x] Runtime degradation: no new service dependencies. Malformed short labels,
  Unicode boundaries, missing public participant/message IDs and unmapped notes
  retain fallbacks; existing geometry regression suite passes.
- [x] Security/config: slice-overlap panic is guarded; no shell/env/file input
  evaluation added. New fixtures use invented data. Advisory gate is green with
  patched dependencies and no additional exceptions; no credentials in the diff.
- [x] Performance: one optional row entry per existing note and note lookup per
  explicit activation; no new external calls, tasks or rendering allocations per
  cell. Existing renderer already scans notes at every message boundary.
- [x] Tests: assertions pin exact output cells and parsed graph structure; a no-op
  renderer or dropped labels/edges fails. Negative tests protect earlier notes
  and inline activation timing. Existing corpus/snapshots pass unchanged.

## Evidence inspected

`gh pr view 58`, `gh pr diff 58`, review/comments APIs, complete affected code,
Cargo.lock before/after package sets, local gate logs, Rust 1.92 log, library
publish dry run, Unicode/ASCII CLI output, CI/release workflows. Local scope
report: docs/scope-open-issues-2026-11-05.md.

## Residual limits

Real WSL/WezTerm display verification for #48 remains external and is explicitly
kept open with user authorization. Nix and non-macOS platform validation run in
CI. Existing unrelated contributor PRs remain outside this release's issue scope.
