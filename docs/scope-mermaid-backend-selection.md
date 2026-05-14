# Scope — Mermaid backend selection and exposure

**Status:** **SHIPPED 2026-05-14** — Phase 1 (config field + wiring +
popup row, surfaced before this scope doc was rediscovered) landed
in `markdown-tui-explorer` 1.34.48-1.34.69 (`MermaidTextBackend`
enum, `mermaid_text_backend` config field, popup section with
Sugiyama/Native rows, `try_text_render` + `try_text_render_with_gaps`
both honouring the chosen backend, load-bearing
`backend_threads_through_render_with_options` and
`modal_gap_render_uses_chosen_backend` regression tests).

Phase 2 (`Auto`) landed in `markdown-tui-explorer` 1.34.70 on
2026-05-14. One deliberate deviation from the proposal below:
**`Auto` is opt-in, not the default.** Sugiyama remains the default;
`Auto` becomes the visible recommendation in the popup but does not
flip rendering for users who haven't touched the setting. Rationale:
the "narrow gating envelope first" rule — give `Auto`'s heuristic a
release of real-world exercise before promoting it to default. The
promotion is queued for a follow-up release once we see no field
reports against the heuristic.

Phase 3+ (tuning + ELK evaluation) remain proposals; revisit only
after field signal.

Original proposal preserved below for context.

---

**Status:** **PROPOSED 2026-04-24.** Expose the two text-mode
backends we already ship (`Native` and `Sugiyama`), add an
`Auto` selector for text mode, and defer any third backend
integration until real diagrams justify it.

This document records:

- what the repo already implements,
- what Mermaid upstream means by its documented "layout algorithms",
- which of those are relevant to this project,
- and the concrete proposal that best fits the current codebase.

## TL;DR

The repo already has two Mermaid layout backends in `mermaid-text`:

- **`Native`**: the in-house default. Full feature coverage for our
  current text renderer, including subgraphs, parallel-edge widening,
  and nested-direction handling.
- **`Sugiyama`**: an opt-in `ascii-dag` adapter. Better crossing
  minimisation and long-edge handling on flat DAGs, but not yet wired
  for subgraphs or parallel-edge groups.

The app does **not** currently expose this choice in user config.
Users can choose Mermaid **mode** (`Auto` / `Text` / `Image`), but not
the text-layout backend.

**Recommendation:**

1. Add a user-facing text backend setting:
   `mermaid_text_backend = "auto" | "native" | "sugiyama"`.
2. Make `auto` the default.
3. Use a simple heuristic:
   - prefer `Sugiyama` for flat, acyclic, no-subgraph dependency graphs,
   - otherwise use `Native`.
4. Do **not** add `cose-bilkent`, `tidy-tree`, or ELK support to
   `mermaid-text` right now.
5. Revisit ELK only if we later decide the image-render path needs
   explicit upstream-Mermaid layout parity for large flowcharts.

---

## Current state in this repo

### App-level Mermaid settings today

The user-facing config currently exposes only:

- `mermaid_mode`
- `mermaid_max_height`

There is no config field for choosing the text layout backend.

Reference:
- `src/config.rs`

### Text-mode Mermaid rendering today

`src/mermaid.rs` calls `mermaid_text::render_with_width()` for normal
text rendering and `mermaid_text::render_with_options()` only for the
modal's explicit gap override path.

References:
- `src/mermaid.rs`
- `crates/mermaid-text/src/lib.rs`

### Backends already implemented in `mermaid-text`

`mermaid-text` already supports:

- **`LayoutBackend::Native`**: default backend, described in the code
  as the stable path that respects every feature we ship.
- **`LayoutBackend::Sugiyama`**: opt-in adapter around `ascii-dag`,
  intended for cleaner layouts on flat dependency graphs.

The trade-off is explicit in the code:

- `Native` supports subgraphs, parallel-edge widening, direction
  overrides, and current renderer expectations.
- `Sugiyama` currently has coverage gaps for subgraphs,
  parallel-edge groups, and nested direction overrides.

References:
- `crates/mermaid-text/src/layout/layered.rs`
- `crates/mermaid-text/src/layout/sugiyama.rs`
- `CHANGELOG.md` (`1.15.0`)

### Image-mode Mermaid rendering today

The image path calls `mermaid_rs_renderer::render(source)` directly.
At the repo level, there is no existing wiring for Mermaid upstream's
`layout: dagre` / `layout: elk` style configuration.

Reference:
- `src/mermaid.rs`

This matters because "should we add ELK?" means something different
depending on which pipeline we are talking about:

- **text mode**: our own `mermaid-text` backends,
- **image mode**: the external SVG renderer crate.

---

## What Mermaid upstream means by "layout algorithms"

Mermaid upstream currently documents four layout names:

- `dagre`
- `elk`
- `tidy-tree`
- `cose-bilkent`

That list is easy to over-read. They are **not** four equal
alternatives for the same diagrams.

### Layered graph layouts

For flowchart/state/class-style diagrams, Mermaid's relevant choices
are layered engines:

- **Dagre**
- **ELK**

In Mermaid's schema, `defaultRenderer` for flowchart/state/class is
`"dagre-wrapper"` by default, with `"elk"` as the advanced option.

### Tree layouts

`tidy-tree` is a tree-specific algorithm. Mermaid's own docs say it is
currently "primarily supported for mindmap diagrams."

This is the Reingold-Tilford / Buchheim family of tidy tree layouts,
good for strict parent-child hierarchies, not general flowcharts.

### Force-directed layouts

`cose-bilkent` is a force-directed compound-graph layout. In Mermaid's
schema it appears as the default `layoutAlgorithm` for **mindmap**
diagrams, not as the general answer for flowcharts.

This class of algorithm is for "organic" graph layouts where global
balance and cluster separation matter more than strict directionality.

## Important conclusion

For this repo's current Mermaid usage, the real categories are:

- **layered DAG / flowchart layouts**
- **tree layouts**
- **force-directed layouts**

Our current text renderer mainly needs the first category.

---

## State of the art by category

### 1. Layered layouts

For directed graphs and flowcharts, the relevant serious options are:

- **ELK Layered**
- **Graphviz `dot`**
- **Dagre / dagre-style implementations**
- **Sugiyama-family implementations**, including our own
  `Native` backend and the `ascii-dag` adapter already in the repo

#### ELK

ELK Layered is the strongest open-source configurable option in this
space. It exposes cycle breaking, layer assignment, crossing
minimisation, node placement, and routing controls.

Best fit:
- large or intricate graphs,
- users who need explicit layout tuning,
- platforms where extra dependency weight is acceptable.

Cost:
- heavier integration,
- more configuration surface,
- farther from the current `mermaid-text` architecture.

#### Graphviz `dot`

`dot` remains a mature baseline for hierarchical graphs. It is still a
credible reference point, but bringing it into this repo would mean a
much larger integration and packaging decision than the current code
needs.

#### `ascii-dag` / Sugiyama

This is the relevant "near-term state of the art" for our text-mode
renderer because it is already integrated and already producing value.
It gives us:

- dummy-node handling for long edges,
- better crossing minimisation,
- Brandes-Kopf-style coordinate assignment,
- terminal-native output geometry.

The blocker is not algorithm quality. The blocker is feature coverage
against our existing `Native` path.

### 2. Tree layouts

For strict trees, the tidy-tree family is the standard answer. D3's
`tree()` documents the Reingold-Tilford tidy tree algorithm with
Buchheim's linear-time improvement.

Best fit:
- mindmaps,
- directory trees,
- rooted hierarchies with one-parent semantics.

Not a fit for:
- general flowcharts,
- cross-links,
- cyclic or DAG-like dependency graphs.

### 3. Force-directed layouts

For organic compound graphs, `cose-bilkent` is established prior art,
but it is no longer the strongest recommendation in practice.
Cytoscape's own guidance recommends **`fcose` first** if you are
choosing a force-directed layout.

Best fit:
- exploratory graph views,
- cluster-heavy undirected graphs,
- interactive graph tools.

Poor fit for this repo:
- deterministic terminal rendering,
- compact reading in a markdown viewer,
- flowchart and state-diagram semantics.

---

## Recommendation

Expose what we already have before adding something new.

That means:

1. add a backend choice for **text mode**,
2. default it to `auto`,
3. keep `Native` as the safe path,
4. opportunistically use `Sugiyama` when the diagram shape matches its
   strengths.

This is the highest-ROI move because:

- the code is already here,
- the value is already proven,
- the risk is bounded,
- and it avoids taking on a large new dependency or architecture
  branch before the current one is fully exploited.

## Why not add ELK now?

ELK is the strongest general-purpose upstream-style answer, but it is
not the right next step here.

Reasons:

- the current gap is mostly **exposure and selection**, not absence of
  layout engines;
- `mermaid-text` already has a second backend with a clear niche;
- ELK would be a third layered backend before we have even surfaced the
  second one to users;
- the repo's own roadmap already defers further compaction work until
  real diagrams prove the need.

ELK becomes sensible later if we want one of these:

- explicit parity with Mermaid upstream's `layout: elk`,
- large-graph image rendering with user-tunable layout controls,
- or a broader re-think of the image-render pipeline.

## Why not add `cose-bilkent` or `tidy-tree` now?

Because they solve the wrong problem for the diagrams we currently care
about.

- `tidy-tree` is for mindmap/tree hierarchy work.
- `cose-bilkent` is for organic force-directed graph layouts.

They are relevant only if we plan to expand text-mode support for
mindmap/tree-style diagrams and want layout selection there.

---

## Proposed product/API shape

## New config field

Add a new app config field:

```toml
mermaid_text_backend = "auto"
```

Allowed values:

- `auto`
- `native`
- `sugiyama`

Suggested enum name:

- `MermaidTextBackend`

Suggested default:

- `Auto`

## Behaviour

### `native`

Always render text-mode flowchart/state diagrams with
`LayoutBackend::Native`.

### `sugiyama`

Always render text-mode flowchart/state diagrams with
`LayoutBackend::Sugiyama`.

If the backend is forced and the diagram hits a known unsupported shape,
we should still render rather than erroring if the current code path can
do so safely. If not, fall back to `Native` and record the reason in a
small internal comment or debug log.

### `auto`

Choose backend by heuristic.

Recommended first heuristic:

- use `Sugiyama` when all are true:
  - diagram kind is flowchart or state-as-flowchart,
  - no subgraphs are present,
  - no parallel-edge groups are present,
  - graph is acyclic,
  - graph has at least one long-spanning edge or enough layering depth
    that crossing-minimised layout is likely to help
- otherwise use `Native`

This keeps `auto` intentionally conservative.

## Scope boundary

This setting should affect:

- text-mode inline Mermaid rendering,
- text-mode Mermaid modal rendering,
- any `mermaid-text` helper paths that already use `RenderOptions`.

It should **not** affect:

- image-mode rendering through `mermaid-rs-renderer`,
- Mermaid source fallback display,
- future non-flowchart diagram-specific layout pipelines unless we
  explicitly extend it.

---

## Implementation plan

### Phase 1 — expose existing backend

Add the config enum and wire it through:

- `src/config.rs`
- `src/app/mod.rs`
- `src/ui/config_popup.rs` if we want it visible in the popup
- `src/mermaid.rs`

Update `try_text_render()` so it calls `render_with_options()` instead
of `render_with_width()` and passes the chosen backend.

This phase alone unlocks manual testing and real-world feedback.

### Phase 2 — add `Auto`

Add a small backend selector in `src/mermaid.rs` or `mermaid-text`
call-site glue.

Keep the heuristic local and readable. Do not hide it behind a large
policy abstraction yet.

### Phase 3 — tune with real diagrams

Only after users exercise the setting:

- broaden `auto` if safe,
- or tighten it if regressions appear,
- or revisit whether `Sugiyama` can become the default for more cases.

### Phase 4 — decide whether a third backend is warranted

Re-evaluate only after we have:

- user feedback on `Native` vs `Sugiyama`,
- examples where both are inadequate,
- and a clear target pipeline (text vs image).

At that point the next candidate is likely:

- **ELK** for image/upstream-parity work,
- not `cose-bilkent`,
- not `tidy-tree`,
- and probably not another layered backend in `mermaid-text`.

---

## Non-goals

This proposal does **not** include:

- adding ELK to `mermaid-text`,
- changing image-mode rendering to honour Mermaid frontmatter layout,
- adding mindmap layout selection,
- adding tidy-tree support to the text renderer,
- adding force-directed layouts,
- or changing the default image renderer crate.

Those are separate decisions.

---

## Risks and mitigations

### Risk: confusing layout names

Users may read Mermaid upstream docs and expect `tidy-tree` or
`cose-bilkent` to apply to ordinary flowcharts.

Mitigation:
- document clearly that `mermaid_text_backend` is a text-renderer
  backend selector, not a full mirror of Mermaid upstream layout names.

### Risk: `Auto` feels unpredictable

If the heuristic is too clever, users will not know why a diagram
changed shape.

Mitigation:
- keep `Auto` conservative,
- keep `Native` and `Sugiyama` available as explicit overrides,
- and document the rule in one short paragraph.

### Risk: exposing `Sugiyama` encourages users to force it on diagrams
it does not yet fully cover

Mitigation:
- call out its intended use in config docs and UI copy:
  "best for flat dependency graphs; use Native for subgraph-heavy
  diagrams."

---

## Strong recommendation

Implement backend exposure now, not a new backend.

Concrete recommendation:

1. add `mermaid_text_backend = auto|native|sugiyama`,
2. make `auto` the default,
3. route `auto` conservatively toward `Sugiyama` only for flat DAGs,
4. leave image-mode and upstream-layout parity unchanged,
5. revisit ELK only after real diagrams prove the current two-backend
   story is insufficient.

This gives the repo a real user-facing improvement with low code churn
and no new dependency decision.

---

## References

### Repo references

- `src/config.rs`
- `src/mermaid.rs`
- `crates/mermaid-text/src/lib.rs`
- `crates/mermaid-text/src/layout/layered.rs`
- `crates/mermaid-text/src/layout/sugiyama.rs`
- `ROADMAP.md`
- `CHANGELOG.md`

### External references

- Mermaid layouts docs:
  https://mermaid.js.org/config/layouts
- Mermaid flowchart config schema:
  https://mermaid.js.org/config/schema-docs/config-defs-flowchart-diagram-config.html
- Mermaid state defaultRenderer schema:
  https://mermaid.js.org/config/schema-docs/config-defs-state-diagram-config-properties-defaultrenderer.html
- Mermaid mindmap config schema:
  https://mermaid.js.org/config/schema-docs/config-defs-mindmap-diagram-config.html
- Mermaid tidy-tree docs:
  https://mermaid.js.org/config/tidy-tree.html
- ELK layered reference:
  https://eclipse.dev/elk/reference/algorithms/org-eclipse-elk-layered.html
- Graphviz layout engines:
  https://graphviz.org/docs/layouts/
- D3 tidy tree:
  https://d3js.org/d3-hierarchy/tree
- Cytoscape layout guidance:
  https://blog.js.cytoscape.org/2020/05/11/layouts/
