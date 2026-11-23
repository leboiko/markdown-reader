# v1.36.0 release validation

Scope: release the reviewed master changes, fix demonstrated cache retention,
and validate memory behaviour. Ceiling: 120 production-code lines, 400 lines of
memory tests/tooling, zero snapshot reclassification or accepted regressions.
The incompatible ascii-dag 0.11 PR #62 remains excluded; see PR-62-review.md.
mermaid-text stays at 0.58.0: its published sources have not changed.

## Findings and repairs

Before changing production code, three lifecycle tests failed on master:
replacing a document retained its old diagram/formula entries, closing the last
tab retained them, and reload left diagram height metadata behind (130 versus
the expected default 20). These are reachable allocations: a native leak scanner
alone does not detect them.

Reuse the existing reload eviction policy after replacement, save, tab close
(keyboard and mouse), and re-render. Keep entries referenced by any open tab;
remove Mermaid height hints with their entries. Regression tests also verify
shared-tab ownership, save, and rejection of a late render completion after close.

Variant search: inspected all `view.load`, `tabs.close`, `tabs.rerender_all`,
cache `retain`/`clear`, and async render completion sites in `src/app`,
`src/mermaid.rs`, and `src/math_image.rs`. Session restore adds tabs before
rendering and needs no eviction. Math already prunes height metadata correctly.
Per-view layout caches reset on load. No broad cache eviction on every frame:
the live-block scan occurs only on document lifecycle changes.

## Repeatable checks

- `scripts/verify-release.sh`: formatting, Clippy (all targets/features), full
  workspace tests on stable and MSRV 1.92, docs, cargo-deny, and cargo-audit.
- `python3 scripts/check-memory.py`: optimized test executable, 2,200 unique
  document replacements and real TUI draws; 500 formula PNG renders; 400 Mermaid
  SVG/raster renders. Exact stale-entry/height assertions complement RSS checks.
  RSS sampled every 100 document iterations; after warmup, spread must be at most
  16 MiB. Then repeat all three workloads under native macOS `leaks --atExit`.
- `scripts/check-memory-asan.sh`: nightly with instrumented standard library,
  full workspace address checks, then isolated soaks with leak detection enabled.
  Install nightly and rust-src first. Tests do not modify real user configuration.

## Results and limits

Apple Silicon macOS, Rust stable 1.99.1, nightly 1.102.0 (2026-11-22):
optimized RSS checkpoints ranged from 13,440 to 13,632 KiB; after warmup the
spread was 160 KiB and the last nine checkpoints were 13,632 KiB. Native leaks
reported **0 leaks / 0 leaked bytes** across all three workloads.

The unsuppressed full-suite LeakSanitizer pass reports 3,353 bytes in 32
allocations through Apple's Network framework and a detached DNS resolver.
A separate application-free reproducer using std::net::ToSocketAddrs in a
thread, sending its result over a channel and exiting without joining, reports
3,327 bytes in 29 allocations; joining that worker produces no report. A tiny
ureq-only reproducer also reports 3,327 bytes. This is a process-shutdown/DNS
worker limitation, not evidence of growing document caches. No blanket leak
suppression is installed: the full suite checks address errors with leak
checking disabled, and the isolated document/raster tests keep leak checking on.
The release is not claimed to be universally leak-free. Tests cover synthetic
inputs and this host; native third-party/system libraries are not fully
instrumented, and terminal-specific graphics protocols are not soaked.

Existing doc warnings and five audit maintenance warnings are unchanged and
allowed by the repository's reviewed policy; no new advisory exception added.

References: [Rust sanitizer documentation](https://doc.rust-lang.org/unstable-book/compiler-flags/sanitizer.html),
[Apple leak detection](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/ManagingMemory/Articles/FindingLeaks.html).

## Release review

No remaining actionable findings in the cache fix. Data safety: eviction only
touches derived caches, never document text or files. Concurrency: live shared
IDs survive, removed pending entries reject late completions. Degradation:
renderer fallback remains covered; failure does not resurrect stale entries.
Security/config: synthetic fixtures and isolated test configuration, no secrets
or shell-interpolated user input. Performance: two linear live-block scans per
lifecycle change, no new per-frame scan. Tests: observed failures before fix,
negative shared-owner/late-completion cases, full suite plus memory workloads.
No schema, network protocol or public API changes.
