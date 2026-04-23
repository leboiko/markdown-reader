# Architecture cleanup plan: 2026-04-23 → mid-May

Five-phase roadmap derived from `docs/markdown-text-research.md` plus the
graph-easy / dagre layout survey. Goal: stop paying interest on the
"predict ratatui's wrap" debt, ship the largest user-visible markdown
gap (wide-table cell wrapping), and finish the mermaid layout-quality
pass that started with 1.20.3 (dummy-node augmentation).

This is an OSS project. Code is the artifact. We optimise for
**clarity, no duplication, and zero dead surface area** — never for
"how fast can I land it."

## Quality gates (every phase, no exceptions)

Each ship must satisfy all of:

- `cargo fmt --all -- --check` — zero diff.
- `cargo clippy --all-targets -- -D warnings` — zero warnings.
- `cargo test --workspace` — all green; new code carries its own tests.
- **No dead code** — every `pub` item has a caller in this repo or a
  documented external use; every helper is reachable. Audited per
  phase; superseded code is *removed*, not left "just in case".
- **No duplication** — width math, span splitting, line measurement
  each live in exactly one place. If a phase produces a near-clone of
  an existing function, the older one is refactored to call the new.
- **Rustdoc on every pub item** — what it does, what invariants it
  upholds, when to use it vs. a sibling.
- **Width-sweep test for any width-aware change** — render the same
  input at widths in `[20, 40, 60, 80, 120, 200]` and assert
  invariants (no overflow, no lost spans, monotonic source-line
  mapping, idempotent re-render).
- CHANGELOG entry per ship; semver-aware version bump; commit message
  describes *why*, not just *what*.

## Phases

### Phase 1 — `text-layout` foundation (1 week)

**Why:** every recent bug (1.18.3 → 1.18.5 cursor saga, link/anchor
visual-row math, doc-search drift) has the same root: width
calculation duplicated across `visual_rows.rs`, `current_line_width`,
table layout, and gutter rendering. One model + one wrapper kills the
class.

**Deliverable:** new module `src/text_layout/` with:

- `WrappedSpan { content, style, width: u16 }` — owned styled chunk
  with cached display width.
- `WrappedLine { spans: Vec<WrappedSpan>, width: u16 }` — single
  visual row.
- `fn wrap_spans(spans: &[Span], width: u16) -> Vec<WrappedLine>` —
  greedy grapheme-aware word wrap; preserves style runs.
- `fn measure(spans: &[Span]) -> u16` — total display width.

**Integration (this phase, not later):**

- `visual_rows::line_visual_rows` and `visual_row_to_logical_in_block`
  reimplemented in terms of `wrap_spans`.
- `current_line_width` reads from cached widths.

**Tests:** width-sweep harness; unit tests for grapheme boundaries,
wide chars, empty lines, multi-style spans; round-trip property test
(wrap → flatten → re-wrap is idempotent).

**Risk:** subtle — must match ratatui's wrap behaviour for the case
where we still hand text to ratatui (gutter `Paragraph`). Phase 3 will
remove that dependency.

### Phase 2 — wrapped-cell tables (1 week)

**Why:** every wide table currently truncates. Largest user-visible
markdown gap. Termimad's pipeline is the proven model.

**Deliverable:**

- `src/ui/table_render.rs` gets a wrapped-row layout path using
  `wrap_spans` from Phase 1.
- Expanded table modal (`src/ui/table_modal.rs`) opts in first; inline
  view follows once snapshots are stable.
- Row height = max wrapped-line count of any cell in that row;
  alignment preserved across the wrap.

**Tests:** width-sweep tables, snapshot tests for the expanded modal,
existing truncation tests still pass for the (configurable) compact
mode.

**Quality:** any width math reaches into Phase 1; existing fair-share
column-width logic stays as-is and gets the wrap pass added on top.

### Phase 3 — own prose wrapping; retire `visual_rows.rs` (1 week)

**Why:** `Paragraph::wrap` is opaque to us; we keep writing parallel
"predict what ratatui will do" code. Owning wrapping deletes a class
of bugs preemptively.

**Deliverable:**

- `DocBlock::Text` gains a cached `Vec<WrappedLine>` populated on
  every layout-width change (mirror of how `TableLayout` works today).
- `draw.rs` renders the wrapped lines directly via a basic
  `Paragraph` (no `.wrap()`), matching their already-correct widths.
- `block.height()` returns the cached length — the visual-vs-logical
  rift introduced in 1.18.4 collapses back into one coordinate space.
- `src/ui/markdown_view/visual_rows.rs` is **deleted**. Its callers
  use `WrappedLine` directly.
- `current_line_width`, `recompute_positions`, `source_line_at_width`,
  `logical_line_at_source_width`, `collect_match_lines` all simpler.

**Tests:** every existing cursor / scroll / highlight / search test
must pass unchanged. Width-sweep tests added for the cursor-on-wrap
case that broke us in 1.18.5.

**This is the largest phase.** Touches the most files. Worth it
because the diff is mostly *deletion*.

### Phase 4 — mermaid-text A\* edge routing (1 week)

**Why:** identified by the dagre+graph-easy survey as the single
biggest visual win for flowcharts. Independent of Phases 1–3 (separate
crate, no shared code).

**Deliverable:**

- `crates/mermaid-text/src/router/` module:
  - `grid.rs` — occupancy map of cells (node interiors, existing
    edges, label boxes).
  - `astar.rs` — A\* with costs `step=1`, `crossing=30`, `turn=6`
    and Manhattan + diagonal heuristic.
- `layered::compute_edge_waypoints` rewritten to call the router;
  the existing handcrafted waypoint logic is **deleted**, not
  switched off behind a flag.

**Tests:** snapshot tests on the README examples + a corpus of dense
graphs; assertion that crossing count never increases on existing
fixtures.

### Phase 5 — `classDiagram` support (1–2 weeks)

**Why:** zero coverage today, third-most-used Mermaid type, common in
architecture docs.

**Deliverable:**

- `crates/mermaid-text/src/types/class.rs` — class, attribute,
  method, relationship types.
- `crates/mermaid-text/src/parser/class.rs` — Mermaid classDiagram
  syntax: `class Name { +field : Type; +method() : Type }`,
  relationships (`<|--`, `*--`, `o--`, `..>`, `..|>`).
- `crates/mermaid-text/src/render/class.rs` — boxes with attribute
  tables (reuses ER's `render::er` table machinery — refactor it into
  a shared helper rather than duplicating).
- `detect.rs` adds `DiagramKind::Class`.

**Tests:** parser unit tests per syntax form; snapshot tests covering
single class, inheritance, composition, multi-class arrangements;
width-sweep.

## Execution model

For each phase:

1. **Plan** — `Plan` agent designs the public API, file layout,
   integration points. I review.
2. **Implement** — `rust-developer` agent does the implementation in
   one focused pass with explicit "no dead code, no duplication"
   constraints in the prompt.
3. **Audit** — `Explore` agent walks the new code looking for
   duplication with existing helpers and unused public items.
4. **Ship** — I run the quality gates, write the CHANGELOG entry,
   bump versions, commit, tag, push.

No phase ships if any quality gate fails. We don't carry forward
"will fix in a follow-up" tech debt — fix it before merging.

## Out of scope (for now)

- **Parser swap** to `comrak` / `markdown-rs`. Research doc §5: only
  if structural complexity becomes the bottleneck. Today layout is.
- **Brandes-Köpf x-coordinates** in mermaid-text. Natural follow-up
  to A\* but is a major lift on its own.
- **Gantt / mindmap / git graph / C4** diagram types. Scheduled
  after Phase 5 ships and the foundation work pays off.

## Success criteria

When Phase 3 ships, the 1.18.3 → 1.18.5 cursor saga becomes
*structurally impossible*: there is exactly one width source of truth.

When Phase 4 ships, every flowchart in the repo's snapshot corpus has
≤ the crossing count it had before.

When Phase 5 ships, mermaid-text covers the three diagram types that
account for ≈80% of real-world Mermaid usage (flowchart + sequence +
class), plus state, pie, and ER.
