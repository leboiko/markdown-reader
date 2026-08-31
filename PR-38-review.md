# PR #38 review — high-fidelity block math via RaTeX

Reviewed: 2026-08-31  
Base: `master` at `a63c995`  
Head reviewed: rebased `feat/35-ratex-math`

## Verdict

Approve after the findings below are committed, pushed, and the repository's
full local/remote gates pass. The architecture matches issue #35 and keeps the
feature opt-in, but the original head was not safe to merge unchanged.

Hard scope ceiling: at most 150 additional production lines beyond the
contributor PR, at most 6 snapshot updates, and zero regressions. The repair is
121 production lines, changes no snapshots, and keeps the contributor commit
and authorship intact.

## Findings

| Severity | Finding | Resolution |
|---|---|---|
| Major | `MathReady` accepted any completion while the same content hash was `Pending`. After a theme refresh or remove/re-add cycle, an older foreground-colour render could overwrite the newer request. | Give each pending render a monotonic request id, carry it in `Action::MathReady`, and accept only the matching completion. A two-completion regression test proves both stale rejection and current acceptance. |
| Major | `math_max_height` was applied only after PNG allocation. A flat, very wide expression could make `tiny-skia` allocate an arbitrarily large RGBA canvas. | Validate natural pixel dimensions before `render_to_png`, with 8192 px per-axis and 16,777,216-pixel area ceilings; oversized formulas fall back with an explicit reason. A 1,000-glyph reproduction failed before the guard and passes after it. |
| Minor | The PR pinned RaTeX 0.1.13 although 0.1.14 is current and adds parser/layout stack-depth guards. | Update the full RaTeX crate family and lockfile to 0.1.14. |
| Minor | `retain` evicted image entries but retained their height bookkeeping indefinitely. | Retain `last_known_heights` against the same live id set and assert an evicted id returns the default height. |

## Review coverage

- Markdown gating, source-byte contiguity, source-line/cursor mapping, hybrid
  reparsing, search, and yank behavior.
- Cache lifecycle across reload, theme and mode changes; stale task delivery;
  lazy lookahead queueing and fallback states.
- RaTeX parse/layout/raster path, transparent/theme-coloured output, allocation
  bounds, terminal protocol/tmux fallback, and configured height limits.
- Config defaults, TOML round-trips, settings routing, background tabs, and
  text↔image cursor-position preservation.
- Dependency versions, licenses/advisories, formatting, clippy, tests, docs,
  locked resolution, and CI/Nix checks.

## Regression evidence

Before the fixes:

- `stale_math_completion_cannot_overwrite_newer_request_for_same_id` ended in
  `"old"` instead of the exact expected `"new"` result.
- `excessively_wide_formula_is_rejected_before_rasterisation` produced a large
  PNG instead of the required safe-limit error.
- `retain_drops_only_absent_ids` returned stale height `12` instead of default
  height `5` after eviction.

The cursor-position test exercises both text→image and image→text transitions,
requires the block representation to actually change, and pins the exact source
line before and after each transition.
