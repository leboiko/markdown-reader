# PR #68 review — v1.36.0

Reviewed head 7a09fdb808d41c6324e76ba92b26a41ff7d03a07 and the surrounding
load/save/reload/close and async-result handlers. A subsequent full-suite leak pass found an existing test-only Box::leak
(26 bytes/three strings). Replaced it with Vec<String>, preserving exact
assertions. Leak-enabled CLI integration also exposed process::exit bypassing
Tokio cleanup; return ExitCode and parse help/version before runtime creation.
The existing integration reproduction verifies the fix without changing output
or exit statuses. No remaining actionable findings in the release diff. Memory evidence and limitations are recorded in
[the release validation report](docs/release-1.36.0.md).

The cache policy belongs to App, which owns all tabs and both render caches.
Extracting the reload policy avoids independent lifecycle-specific policies;
Mermaid metadata now follows the same lifetime as entries. The two live-ID
sets cost O(open document blocks) only on lifecycle changes. No public API,
file format, editor text or rendering geometry changes.

## Red-Team Checklist

- Data safety: checked. All eviction is derived state. Failed/stale loads return
  before changing the tab or pruning; saved content and dirty baselines retain
  existing semantics. No database/schema/migration surface.
- Concurrency: checked. Shared content IDs remain while any tab references them.
  Pending results for removed IDs are rejected by existing completion guards;
  exact late-result and shared-owner assertions cover the changed lifecycle.
- Degradation: checked. Closed/replaced entries are removed independent of
  successful rendering. Existing render-failure fallback and all four SVG
  renderer fixtures remain exercised. No new external service dependency. CLI teardown now drops the runtime on all
  normal early-exit paths; parse errors/help/version occur before workers start.
- Security/config: checked. No unsafe code, dynamic SQL, external destination,
  permission or secret changes. Memory fixtures are synthetic; test config is
  isolated. Native leaks uses --noContent. Scripts check subprocess exit codes.
- Performance: checked. No added per-frame scan, timeout or thread. Cache growth
  is bounded by live document references, with metadata included. RSS soak also
  catches allocator growth that exact ownership tests cannot observe.
- Tests: checked. Three pre-fix failures reproduced actual retained allocations
  or metadata; four regular regressions plus three opt-in soaks guard the fix.
  Shared entries must survive until last close, so indiscriminate clear fails.
  Stable/MSRV full suites and release CLI positive/negative smoke pass.

Residual limits: synthetic workload on Apple Silicon, no sustained terminal
image-protocol soak or exhaustive hostile-input fuzzing. Full-suite LSan's
macOS detached-DNS shutdown report is reproduced in a std-only program; it is
not silently treated as a clean run. Full address checking, full-suite leak checking excluding only that DNS
test, and isolated unsuppressed leak checking are separate gates. Final release gate results are
recorded in the linked report before publishing.

Recommendation: merge after final sanitizer and remote checks pass. No
follow-up issue or snapshot reclassification required.
