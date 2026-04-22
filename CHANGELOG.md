# Changelog

All notable changes to `markdown-tui-explorer` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.16.3] - 2026-04-22

### Fixed

- **Edge labels for parallel and multi-outgoing edges stack
  cleanly off the arrow row** (via `mermaid-text` 0.14.2).
  Visible on the README CI/CD pipeline (`pass` above the arrow,
  `skip` below) and the canonical TD state machine
  (`done`/`error` share a single row instead of stacking).
  Free upgrade.

## [1.16.2] - 2026-04-22

### Fixed

- **`mermaid-text` README's "Demo" Input/Output section no longer
  double-renders the same diagram.** 1.16.0's auto-detect was
  catching the Input block (`graph LR; A → B → C → D`) and
  rendering it as Mermaid even though it was meant to display the
  literal source. Tagged the Input as ` ```text ` so it stays raw,
  paired with the existing Output block that shows the rendered
  result.

## [1.16.1] - 2026-04-22

### Removed

- **Dropped the per-block "Rendered output" dogfood code blocks
  from `crates/mermaid-text/README.md`.** They were added in 1.16.0
  to make the README readable in viewers without Mermaid support,
  but in viewers that do render Mermaid (the TUI's auto-detect, our
  own image pipeline, GitHub web) every diagram appeared twice —
  once rendered, once as text below. The dogfood goal is better
  served by the existing CLI quickstart (`mermaid-text < diagram.mmd`)
  and the architecture-diagram comparison block (which stays — it
  showcases the sugiyama backend's alternative output, not a
  duplicate of the mermaid source).

## [1.16.0] - 2026-04-22

### Added

- **Untagged ` ``` ` fences whose first line declares a Mermaid
  diagram now auto-render as Mermaid blocks** (instead of falling
  through to plain code-block display). The detection is tight to
  avoid false positives:
  - `graph` / `flowchart` must be followed by an explicit direction
    token (`TD`, `TB`, `BT`, `LR`, `RL`).
  - Other declarations (`sequenceDiagram`, `stateDiagram-v2`,
    `erDiagram`, `pie`, `gantt`, `journey`, `mindmap`, `timeline`,
    `quadrantChart`, `classDiagram`, `gitGraph`, `requirement`,
    `C4*`) must be the entire first line, with documented
    exceptions for `pie title`, `pie showData`, `gantt dateFormat`.
  - Plain code with a leading `graph = {}` or natural prose like
    `"sequenceDiagram is great"` stays a code block.

  Catches the common authoring mistake of writing ` ``` ` instead
  of ` ```mermaid `, which silently broke rendering of two diagrams
  in `mermaid-text`'s own README until 1.16.0. Both readme blocks
  now also have explicit `mermaid` tags as belt-and-suspenders.

### Changed

- **`mermaid-text` README ships with rendered text-output blocks
  below every Mermaid example.** The README now eats its own dog
  food — every diagram source is followed by the text-mode output
  `mermaid-text` produces, so the README reads correctly in any
  viewer (GitHub, terminal, plain-text grep) regardless of whether
  the viewer renders Mermaid.

## [1.15.1] - 2026-04-22

### Fixed

- **Sugiyama-backend chrome glitches reduced** (via `mermaid-text`
  0.14.1). The architecture-diagram opt-in now has wider inter-
  layer gaps and cleaner junctions. Free upgrade.

## [1.15.0] - 2026-04-22

### Added

- **Sugiyama layout backend (opt-in)** for flat dependency graphs
  (via `mermaid-text` 0.14.0). The mermaid-text CLI gains a
  `--sugiyama` flag and `RenderOptions::backend` for embedded
  callers. Better crossing minimisation + Brandes-Köpf coordinate
  assignment + long-edge dummy nodes via the [`ascii-dag`] crate.
  Default behaviour unchanged — `Native` remains the default
  backend until subgraph and parallel-edge support land in the
  Sugiyama wrapper.

### Changed

- MSRV bumped to 1.92 to match `ascii-dag`'s minimum.

[`ascii-dag`]: https://crates.io/crates/ascii-dag

## [1.14.0] - 2026-04-22

### Fixed

- **Subgraph labels in mixed-direction diagrams have breathing room
  from the border** (via `mermaid-text` 0.13.0). Phase 3 of the
  parallel-edge work: `direction TB` subgraphs inside an `LR` graph
  (and vice versa) widen their bounds when they contain
  parallel-edge labels, with the layered layout pre-allocating the
  same extra space so external nodes don't collide. Visible on the
  README Supervisor (`creates`/`panics`) example. Free upgrade.

## [1.13.5] - 2026-04-22

### Fixed

- **TD/BT state diagrams with cycles render their back-edge entry
  cleanly** (via `mermaid-text` 0.12.2). The garbled `├┤` glyph
  pair at the back-edge source is now a proper L-corner (`├┘`
  for TD, `├┐` for BT). Visible on the canonical README state
  machine. Free upgrade.

## [1.13.4] - 2026-04-22

### Fixed

- **erDiagram relationships now visually connect their entity boxes**
  (via `mermaid-text` 0.12.1). The cardinality glyphs and label
  used to float in a detached row below both boxes — readers had
  to mentally connect them to the entities above. Now the line
  sits at the entity-name row of both boxes, merging into the
  side borders via `┤` and `├` tee glyphs. The README CUSTOMER↔ORDER
  example reads as a single diagram instead of two stacked artefacts.
  Free upgrade.

## [1.13.3] - 2026-04-22

### Fixed

- **Cramped parallel-edge labels in flowcharts and state diagrams
  finally have breathing room** (via `mermaid-text` 0.12.0). When
  two or more labelled edges connect the same node pair (CI/CD's
  `pass`/`skip`, Supervisor's `creates`/`panics`, state diagrams
  with `done`/`task` bidirectional pairs), the inter-layer gap
  now widens to give each label its own row (LR/RL) or column
  (TD/BT). Closes ROADMAP items #2 + #4. Free upgrade.

## [1.13.2] - 2026-04-22

### Fixed

- **State diagrams with back-edges read much more clearly** (via
  `mermaid-text` 0.11.2). The back-edge in cyclic diagrams (most
  TD state machines) now routes around the perimeter instead of
  threading through the diagram body — forward edges and their
  labels stay in clean channels. Free upgrade.

## [1.13.1] - 2026-04-22

### Changed

- **`erDiagram` visual polish** (via `mermaid-text` 0.11.1). Phase
  2 of the erDiagram series: entity boxes now render with attribute
  tables inside (type / name / keys columns), and relationship
  arrows carry single-character cardinality glyphs at each endpoint
  (`1`, `?`, `+`, `*`). Free upgrade.

## [1.13.0] - 2026-04-22

### Added

- **`erDiagram` support** in markdown mermaid blocks (via
  `mermaid-text` 0.11.0). The most-requested missing diagram
  type per ROADMAP now renders natively. Phase 1 — entity-name
  boxes in source-order row, relationships drawn as labelled
  arrows with `1:N` style cardinality summaries, dashed lines for
  non-identifying (`..`) relationships.
- Phase 2 (attribute tables + crow's-foot cardinality glyphs)
  and Phase 3 (grid layout) ship in subsequent `mermaid-text`
  0.11.x releases. Free upgrade — no markdown-reader code
  changes.

## [1.12.1] - 2026-04-22

### Changed

- **Crossing-minimisation hardening** in flowchart and state
  diagrams (via `mermaid-text` 0.10.1). Adds median + transpose
  passes alongside the existing barycenter sweep — no visible
  change on the current gallery (barycenter alone was already
  optimal on these diagrams) but produces tighter layouts on
  pathologically dense graphs that older code would settle into
  sub-optimal local minima. Free upgrade.

## [1.12.0] - 2026-04-22

### Changed

- **Long-edge routing in flowchart and state diagrams** (via
  `mermaid-text` 0.10.0). Edges spanning more than one layer now
  get per-intermediate-layer waypoints, giving them a near-
  straight channel through the layout instead of detouring
  around intervening nodes. Phase A.1 of the layered-layout
  improvements series; Phases A.2 (Brandes-Köpf compaction) and
  A.3 (median + transpose crossing min) ship in subsequent
  `mermaid-text` 0.10.x releases.
- **Source-breaking for external consumers of `mermaid-text`**:
  `layered::layout` now returns `LayoutResult` instead of a
  position `HashMap`; `render::render` gains a fourth parameter
  for waypoints. No surface-level changes in markdown-reader
  itself — bumped to 1.12.0 to reflect the dep's minor bump.

## [1.11.7] - 2026-04-22

### Changed

- **Sequence-diagram polish** in markdown mermaid blocks (via
  `mermaid-text` 0.9.7): bottom participant boxes mirror the top
  (matches Mermaid's bracketed-lifeline convention), and block
  tags split into two `[…]` brackets (`╔═[alt]══[cache hit]═══╗`
  instead of `╔═[alt: cache hit]═══╗`) to match Mermaid's
  badge-plus-condition style. Free upgrade — no markdown-reader
  code changes.

## [1.11.6] - 2026-04-22

### Changed

- **Mermaid TD/BT diagrams: arrow tips merge into destination box
  borders** (via `mermaid-text` 0.9.6). Previously `▾` sat one row
  above each `┌────┐` top border, creating a visible gap in TUI
  display. Now renders as `┌─▾─┐` — the arrow visually connects
  to the box. LR/RL flows already had no gap (cell adjacency).
  Free upgrade — no markdown-reader code changes.

## [1.11.5] - 2026-04-22

### Fixed

- **Edge labels no longer puncture node or subgraph borders** in
  flowchart and state diagrams (via `mermaid-text` 0.9.5). The
  Supervisor pattern's `panics` label inside Factory's bottom
  border, the keyboard-lock state diagram's `EvNumLockPressed`
  overwriting node corners, and similar issues across five state-
  diagram snapshots are all fixed. Free upgrade — no
  markdown-reader code changes.

## [1.11.4] - 2026-04-21

### Added

- **`pie` chart support** in markdown mermaid blocks (via
  `mermaid-text` 0.9.4). First new diagram type since
  `sequenceDiagram`. Renders as a horizontal bar chart with
  optional title and optional `showData` value column. Free
  upgrade — no markdown-reader code changes.

## [1.11.3] - 2026-04-21

### Added

- **Sequence-diagram block statements** in markdown mermaid blocks
  (via `mermaid-text` 0.9.3). `loop`/`alt`/`opt`/`par`/`critical`/
  `break` and their continuation keywords (`else`/`and`/`option`)
  render as labelled rectangles spanning the columns of inner
  messages, with proper nesting and inset for nested blocks.
  Completes the four-part sequence-diagram polish series. Free
  upgrade — no markdown-reader code changes.

## [1.11.2] - 2026-04-21

### Added

- **Sequence-diagram activation bars** in markdown mermaid blocks
  (via `mermaid-text` 0.9.2). Both `activate X` / `deactivate X`
  directives and the inline `A->>+B` / `B-->>-A` shorthand render
  as heavy `┃` overlays on participant lifelines. Free upgrade —
  no markdown-reader code changes.

## [1.11.1] - 2026-04-19

### Added

- **Sequence-diagram notes** in markdown mermaid blocks (via
  `mermaid-text` 0.9.1). `note left of X : text`,
  `note right of X : text`, `note over X : text`, and the
  multi-anchor `note over X,Y : text` form all render now —
  rounded boxes anchored to participant columns. `<br>` /
  `<br/>` in note text becomes a real line break. Free upgrade —
  no markdown-reader code changes.

## [1.11.0] - 2026-04-20

### Added

- **`autonumber` directive in mermaid sequence diagrams** (via
  `mermaid-text` 0.9.0). API call sequences in markdown files now
  show `[1]`, `[2]`, `[3]` … prefixes when the source has
  `autonumber`. Mid-diagram re-base (`autonumber 100`) and pause
  (`autonumber off`) both honoured. Free upgrade — no
  markdown-reader code changes.
- Foundation data model for the rest of sequence-diagram polish
  (notes, activation bars, block brackets); those features land
  in subsequent `mermaid-text` 0.9.x releases.

## [1.10.1] - 2026-04-20

### Added

- **Notes anchored to states** in mermaid state diagrams (via
  `mermaid-text` 0.8.1). `note left of X : text`,
  `note right of X : text`, `note over X : text`, plus the
  multi-line `note left of X / … / end note` form. Each note
  renders as a small rounded box connected to its anchor by a
  dotted, no-arrow line. Free upgrade — no markdown-reader code
  changes.

## [1.10.0] - 2026-04-20

### Added

- **`classDef`, `class`, and `:::className` shorthand** for both
  mermaid flowcharts and state diagrams (via `mermaid-text` 0.8.0).
  Define a colour palette once with `classDef cache fill:#234,…`
  then apply it across many states with `class A,B,C cache` or
  inline (`A:::cache --> B:::warn`). Subgraphs / composite states
  coloured via `class CompositeId styleName` get a coloured
  border. Free upgrade — no markdown-reader code changes; the
  call into `mermaid_text::render_with_width` already passes
  `--color` through.
- **`style` and `linkStyle` now apply to state diagrams** (they
  worked for flowcharts since 0.4.0; were silently skipped for
  state diagrams until now).

## [1.9.2] - 2026-04-20

### Added

- **State diagrams now render `<<choice>>`, `<<fork>>`, and
  `<<join>>` shape modifiers** (via `mermaid-text` 0.7.2). Choice
  points show as decision diamonds; fork / join synchronisation
  bars render as thick lines perpendicular to the flow direction
  (vertical `┃` in LR layouts, horizontal `━━━` in TB). State
  diagrams with branch points (auth flows, Sagas,
  retry-with-conditional) and parallel-flow synchronisation (CI
  orchestration, distributed fan-out / fan-in) now read correctly
  instead of as a chain of identical rounded boxes.

## [1.9.1] - 2026-04-20

### Fixed

- **Edge labels in mermaid diagrams no longer overwrite node interior
  text.** Picks up `mermaid-text` 0.7.1 which expanded the label
  placement candidate set and added a node-interior collision check.
  The user's circuit-breaker FSM rendering used to show a stray `5`
  inside the OPEN state (from the edge label `5 consecutive failures`
  spilling onto the box content); now the label lands on a clean row
  below the segment and OPEN's content is intact.

## [1.9.0] - 2026-04-20

### Changed

- **Mermaid state diagrams now default to `LR` direction.** In a text
  canvas, TB (Mermaid's spec default) inserts `layer_gap` blank rows
  between each row of nodes, so a typical 4-state chain balloons into
  40+ rows — most of it empty. LR keeps the chain horizontal. The
  user's circuit-breaker FSM drops from ~52 rows to ~11 rows. Users
  who want the old layout can add `direction TB` to the diagram
  source. Bumps `mermaid-text` to 0.7.0.

## [1.8.2] - 2026-04-20

### Fixed

- **Scrolling inside a tall mermaid diagram now works.** v1.8.1 stopped
  the layout from clamping the reserved height, but the text-mode
  renderer (`AsciiDiagram`, `SourceOnly`, `Failed`) still always drew
  the diagram from line 0 of the text — `Paragraph::new(text)` ignores
  scroll offsets — so the user saw the top of the diagram pinned in
  place no matter how far they scrolled into it. Now the renderer
  slices the diagram lines by the scroll offset before passing them to
  `Paragraph`, mirroring the `DocBlock::Text` path. Tall composite
  state diagrams scroll smoothly through their full height.

## [1.8.1] - 2026-04-20

### Fixed

- **Tall mermaid diagrams are no longer cut off.** Text-mode diagrams
  (the `AsciiDiagram` cache variant — anything rendered through
  figurehead / `mermaid-text`) used to be clamped to
  `mermaid_max_height` (default 30 lines) when sizing their layout slot.
  A composite-state diagram or any flowchart taller than 30 lines had
  its bottom rows silently unreachable: scrolling moved past the
  reserved region into the next document block instead of revealing
  more of the diagram. Layout now reserves the diagram's actual line
  count, with a 1000-line defensive safety cap. `mermaid_max_height`
  still applies to image renders and source-text fallbacks where the
  bound is meaningful.

## [1.8.0] - 2026-04-20

### Added

- **Mermaid state diagrams now render inline.** `stateDiagram` and
  `stateDiagram-v2` blocks in markdown files are rendered as Unicode
  box-drawing art (previously fell back to showing the raw source).
  Includes `[*]` start/end markers, transitions with labels,
  `STATE : description` accumulation, `state "Display" as Id`, and
  per-diagram direction overrides.
- **Composite states `state X { … }`** with recursive nesting and
  per-composite `[*]` scope render as nested rounded rectangles.
  External edges to / from composite IDs are automatically rewritten
  to land on the composite's inner start / end marker so the arrow
  connects visibly to the composite border region.
- Bumped `mermaid-text` dependency to **0.6.0**.

### Fixed

- **Back-edge perimeter paths now visibly connect to their boxes.**
  Any flowchart (or state diagram) with a back-edge (`C --> A` when
  `A` is upstream of `C`) previously rendered the perimeter line and
  arrow tip with a 1-cell gap to each node's border. `mermaid-text`
  0.6.0 stamps `┬`/`┴` (or `├`/`┤` for TD/BT) junction glyphs at both
  ends so the connection reads cleanly. Surfaces constantly in retry
  loops in state diagrams.

## [1.7.1] - 2026-04-17

### Added
- **`mermaid-text` library crate** (`crates/mermaid-text/`). A standalone
  MIT Rust library that renders Mermaid flowcharts as Unicode box-drawing
  text — no browser, no image protocols, pure Rust. Supports
  `graph`/`flowchart` with LR/TD/RL/BT directions, node shapes
  (rectangle, rounded, diamond, circle), edge labels, and Sugiyama-style
  layered layout. Published as a workspace member; will be released as
  an independent crate.
- **Text-mode mermaid rendering** via `mermaid-text`. Flowcharts in
  Text mode or on non-graphics terminals render as Unicode art instead
  of raw source. Sequence/state/class diagrams still fall back to source
  (Phase 2-3 of `mermaid-text`).
- **Visible block cursor** at `(cursor_line, cursor_col)`. A single-cell
  highlight in `accent` colour shows the exact horizontal position in
  both normal and visual modes, making `h`/`l` movement and `v`
  character selection visually trackable.

### Fixed
- **Mermaid cache invalidated on resize.** Cached `AsciiDiagram` text
  is fixed-width; resizing the viewer now clears the mermaid cache so
  diagrams re-render at the new width.
- **Flowchart parser skips mermaid keywords.** `subgraph`, `direction`,
  `end`, `style`, `classDef`, `click`, `linkStyle` are no longer
  treated as node definitions. `<br/>` tags are stripped from labels.

## [1.7.0] - 2026-04-17

### Added
- **Mermaid rendering settings.** Press `c` → Mermaid section to choose
  Auto / Text / Image rendering mode. `mermaid_max_height` in
  config.toml caps diagram height (default 30 lines, was hardcoded 50).
- **`has_limited_rendering` diagrams (state diagrams) now try
  text-mode rendering** instead of falling through to raw source.
  Infrastructure for `AsciiDiagram` cache variant is in place; the
  text renderer is currently stubbed (the only candidate — figurehead
  0.4.3 — has fatal bugs for TUI use: debug prints, panics, freezes).

### Fixed
- **Link picker (`f`) now updates the cursor.** Selecting a heading
  via `f` jumped the scroll but left `cursor_line` at its old position.
  The next `j`/`k` would snap back to the pre-jump location. Now uses
  `cursor_line + scroll_to_cursor_centered` like all other jumps.
- **Stale mermaid image results no longer overwrite text-mode entries.**
  After switching rendering mode, in-flight image tasks that complete
  are discarded if the cache entry is no longer `Pending`.

## [1.6.4] - 2026-04-17

### Fixed
- **Mermaid renders no longer peg the CPU.** Added a 30-second timeout
  per render and a cap of 2 concurrent render tasks.
  `mermaid-rs-renderer` is pre-1.0 and can hang on certain diagram
  types; previously a hung render would run forever at 100% CPU.  With
  multiple diagrams queued (e.g. after a theme change clears the
  cache), every core could be saturated.  Now: hung renders time out
  cleanly (the diagram shows an error footer), and at most 2 render
  threads run simultaneously.

### Changed
- **Compact tree indentation.** Reduced per-level indent from 2 spaces
  to 1 space and switched expand/collapse markers from `▼`/`▶` to
  the narrower `▾`/`▸`.  At depth 5, filenames now start 5 characters
  earlier — enough to show the full name on most terminals instead of
  truncating.

## [1.6.2] - 2026-04-17

### Fixed
- **Duplicate key events on Windows.** crossterm emits both
  `KeyEventKind::Press` and `KeyEventKind::Release` for every keystroke
  on Windows; the event loop was forwarding both, causing every action
  to fire twice. Now only `Press` events are forwarded. No effect on
  macOS/Linux (they only emit `Press`). Fixes #1.

## [1.6.1] - 2026-04-17

### Changed
- **Code quality: zero clippy pedantic warnings.** Eliminated all 181
  pedantic lint warnings across the codebase: 62 integer-cast warnings
  resolved via new saturating-cast helpers in `src/cast.rs`
  (`u32_sat`, `u16_sat`, `u16_from_u32`); 19 infallible casts replaced
  with `From` trait calls; remaining 100 warnings fixed mechanically
  (redundant closures, `let...else`, inlined format vars, merged match
  arms, items-before-statements, etc.).
- **Module split: `app.rs` (4093 lines) → `src/app/` (7 files,
  largest 1009 lines).** Key handlers, search, file operations, yank,
  table-modal logic, and tests each live in focused submodules.
  `App` struct and top-level dispatch stay in `mod.rs`.
- **Module split: `markdown_view.rs` (2000 lines) → `src/ui/markdown_view/`
  (8 files, largest 528 lines).** Draw, state, highlight, mermaid draw,
  gutter, visual-row math, and tests each in their own file.
- **All production `unwrap()` calls replaced** with `let Some(...) else { return }` guards.

## [1.6.0] - 2026-04-17

### Added
- **Character-wise visual mode (`v`).** Press `v` in the viewer to
  start a character-level selection. `h`/`l`/`Left`/`Right` move the
  cursor horizontally within the line; `j`/`k`/`d`/`u`/`gg`/`G` move
  vertically and clamp the column to the new line's width. `y` yanks
  the exact character range to the clipboard; `Esc`/`v` cancels.
  First/last lines of the selection are partially highlighted; middle
  lines are fully highlighted. Spans are split at column boundaries
  preserving per-span styles.
- **Horizontal cursor (`cursor_col`).** The viewer now tracks a
  column position within the current logical line. `h`/`l` move it
  left/right. The status bar shows `col N` so the position is always
  visible.
- **Line-wise visual mode is now `V`** (uppercase, was also `V`
  before) and shows `VISUAL LINE` in the status bar. `v` (lowercase)
  is character-wise and shows `VISUAL`. Matches vim convention.

### Changed
- `VisualRange` now carries `mode` (`Char`/`Line`), `anchor_col`,
  and `cursor_col` fields alongside the existing line fields.
  `char_range_on_line` is the single method callers use to determine
  highlighting — no mode-branching in the rendering pipeline.

## [1.5.3] - 2026-04-17

### Fixed
- **Search-jump now lands on the correct line.** `logical_line_at_source`
  was returning the *last* logical line whose source number matched the
  target, but the same source line can appear at multiple rendered
  positions (heading + trailing blank, list End-event dip back to the
  list's start line). The last occurrence is a rendering artifact; the
  first is the actual content. Now exact matches return the first
  occurrence immediately. Approximate matches (target inside a joined
  paragraph) still scan the full vector for the closest preceding line.

## [1.5.2] - 2026-04-17

### Fixed
- **Cursor no longer jumps back to line 1 on Linux.** On Linux,
  `inotify` fires `IN_ACCESS` events when a file is read (not just
  modified). Our 500ms-debounced file watcher treated those as changes,
  triggering a reload that reset the cursor and scroll to 0. Now
  `reload_changed_tabs` compares the new content against the existing
  `tab.view.content` and skips the reload when nothing actually changed.
  Genuine reloads also preserve the cursor position (clamped to the new
  document length) instead of always resetting to line 1.
- **`markdown-reader path/file.md` now opens the file immediately.**
  Previously, passing a file path (instead of a directory) produced an
  empty tree because the app used the file itself as the tree root.
  Now the root is set to the file's parent directory, the tree is
  populated normally, and the file is opened in a tab on startup.
- **Borderless viewer when the file tree is hidden.** Pressing
  `Shift+H` to hide the tree now also removes the viewer's outer
  border, giving the markdown content full terminal width and height.
  `[` and `]` (tree width adjustment) are no-ops while the tree is
  hidden. Pressing `Shift+H` again restores both the tree and the
  border.

### Changed
- `App::new` now takes an optional `initial_file: Option<PathBuf>`
  parameter for the file-path-as-argument feature.

## [1.5.1] - 2026-04-17

### Fixed
- **File-tree discovery is dramatically faster on large repos.** The
  recursive per-directory walker (`max_depth(1)` + re-recurse) was
  re-reading and re-compiling `.gitignore` matchers at every directory
  level, which scaled pathologically on monorepos with deep trees.
  Replaced with a single `ignore::WalkBuilder::build_parallel()` pass
  that amortises the ignore-matcher cost across worker threads, then
  folds the flat path list into a sorted `FileEntry` tree.

## [1.5.0] - 2026-04-17

### Added
- **LaTeX math rendering.** Inline math (`$...$`) and display math
  (`$$...$$`) are now parsed via pulldown-cmark's `ENABLE_MATH` option
  and rendered as Unicode-approximated text. Greek letters (`α`, `β`,
  `π`, …), operators (`∑`, `∫`, `∇`, `∞`, …), fractions (`a/b`),
  square roots (`√(x)`), and super/subscripts (`x²`, `xᵢ`) display
  as readable Unicode. Display math renders in a bordered block
  labelled `math`, mirroring the code-block style. Zero new
  dependencies — pure Rust string conversion in `src/markdown/math.rs`.

## [1.4.3] - 2026-04-16

### Fixed
- **Table modal preserved only the first span's colour when slicing for
  horizontal scroll.** The first span on every row is the left border
  `│` styled with `table_border`, so the whole row — including cell
  text and header text — inherited the border's muted colour, making
  the modal unreadable on every theme. `slice_line_at` now walks the
  line span-by-span, keeping each span's original style, and only
  replaces a span's content with the correct display-width slice.
  Double-width characters straddling the left edge are still
  replaced with a single space so column alignment stays consistent.

## [1.4.2] - 2026-04-16

### Changed
- **Trimmed transitive dependencies.** Dropped `image-defaults` from
  `ratatui-image` and `default-features` from `image` — we only use the
  `RgbaImage`/`DynamicImage` types to shuttle pixels from `tiny_skia`
  (mermaid rasterization) to `ratatui-image`, never to decode image
  files. Removing the format decoders also removes the
  `ravif → rav1e → bitstream-io → core2` chain that was triggering a
  "yanked dependency" warning on every build. Significantly smaller
  compile time and binary. No functional change.

## [1.4.1] - 2026-04-16

### Fixed
- **`Enter` now expands the table under the cursor** rather than the first
  table that happens to intersect the viewport.  Falls back to the
  first-visible table when the cursor is on prose, preserving the old
  "click anywhere to expand" behaviour.
- **Table modal contrast** — the expanded-table modal's grid borders
  were rendered with a colour tuned for the main viewer background but
  drawn against the modal's tinted background, which made the grid
  barely visible on light themes (GitHub Light in particular).  The
  modal body now uses the viewer background directly; the focused-border
  colour around the outer frame still signals "this is a modal".

### Changed
- README now includes screenshots (viewer overview, global search,
  GitHub Light with settings) and lists all eight themes in the
  Features section (Solarized Light and Gruvbox Light were missing from
  the count).  The settings-modal keybinding description mentions the
  new "search preview" option.

## [1.4.0] - 2026-04-16

### Added
- **Global search modal.** Press `/` in the Tree or Viewer to open a
  full-screen search pane. Results are grouped per file with a match
  count and a preview of the first match (full-line or ~80-char
  snippet, selectable in Settings). `j`/`k`/arrows/`Ctrl+n`/`Ctrl+p`
  navigate; `Enter` opens the selected file in a new tab; `Tab`
  toggles between Files and Content modes; `Esc` dismisses. Click a
  row to open it, click outside to dismiss.
- **Smartcase search.** Lowercase query = case-insensitive match;
  any uppercase character in the query = case-sensitive. An `Aa`
  / `aA` indicator in the modal footer shows the active mode. No
  manual toggle required.
- **Jump to match line on open.** Confirming a content-search result
  opens the file and places the viewer cursor on the first-match
  source line, centred in the viewport.
- **Tree auto-expand on open.** Whenever a file is opened
  programmatically (search, link follow, session restore), the file
  tree expands any collapsed ancestor directories so the file's row
  is visible and selected.
- **Vim-style visual-line mode in the viewer.** Press `V` to start a
  line-wise selection; `j`/`k`/`d`/`u`/`gg`/`G`/`PageDown`/`PageUp`
  extend the range; `y` yanks the selection to the clipboard via
  OSC 52 and exits; `Esc` or `V` cancels. Status bar shows
  `VISUAL` while active. `yy` in normal mode copies the current
  cursor line.
- **Search preview setting.** New `Search preview` section in the
  Settings modal toggles between Full line (default) and Snippet
  (~80 chars) previews in the search modal. Persisted in
  `config.toml` as `search_preview`.
- **Cursor position in the status bar.** The status bar now shows
  `(cursor_line / total_lines, percentage)` so `d`/`u`/`gg`/`G`
  navigation is reflected immediately. (Already shipped in 1.3.0;
  this release adds the `VISUAL` label override.)

### Fixed
- **GitHub Light theme: invisible tab and status-bar labels.** The
  `accent` and `selection_fg` colors in the GitHub Light palette
  were both the same blue, so text drawn on an accent background
  (active tab name, focus indicator) rendered blue-on-blue and
  vanished. A new `Palette::on_accent_fg` field disambiguates the
  two roles; for GitHub Light it's set to white.
- **Search-jump to the right source line inside lists and
  paragraphs.** Previously the inverse source-to-logical mapping
  assumed `source_lines` was monotonically non-decreasing, but
  pulldown-cmark's End-of-list events can cause dips (e.g.
  `[..., 165, 160, 167, ...]`), leading to wrong jumps for any
  match whose target line lived after a list. The scan now walks
  the full vector and returns the last candidate whose source
  `<= target`.
- **Gutter line numbers now align with wrapped content.** The
  gutter paragraph previously rendered one number per logical
  line against a wrapping content paragraph, so the two drifted
  vertically on long lines. The gutter now emits blank
  continuation rows that match the content's wrap count, so a
  line number always sits next to its content.
- **Table header source-line tracking.** pulldown-cmark does not
  emit `Tag::TableRow` for a table's header — cells live directly
  inside `Tag::TableHead` — so the header's source line was
  recorded as 0 regardless of the table's actual position. Now
  captured from `Tag::TableHead`'s own span.
- **`pending_jump` no longer leaks on read failure.** A new
  `Action::FileLoadFailed` variant fires when the async read
  fails, clearing the pending jump so a later unrelated file
  load cannot inherit a stale target.
- **Misleading search-truncation footer.** The "N more" count was
  derived by subtracting a file cap from a match count. Replaced
  with a clear `"results capped at N files"` message.

### Changed
- **`:N` go-to-line now centres the target** to match the UX of
  search-result jumps. Both are long-distance jumps; neither
  should park the cursor at the viewport edge.
- **Content search counts all matches per file.** Previously the
  search broke after the first match in each file; the new
  modal needs the count for its per-file display.
- **`edtui` upgraded to 0.11.2** (already in 1.2.0) now with
  `default-features = false` to drop the `arboard` clipboard
  dependency we do not use. Smaller binary, headless-safe.

## [1.3.0] - 2026-04-15

### Fixed
- **Doc-search navigation now moves the viewer cursor.** `n`/`N` and the
  auto-jump to the first match were mutating `scroll_offset` directly,
  leaving `cursor_line` stranded at its old position. Press `j`/`k`
  after `n` now moves the cursor from the match row, as expected.
- **Cursor highlight no longer disappears over tables and mermaid
  blocks.** The highlight code now runs for `DocBlock::Text`,
  `DocBlock::Table`, and the source-text fallback of `DocBlock::Mermaid`
  via a shared `patch_cursor_highlight()` helper. Mermaid blocks in
  image mode render a 1-row background bar beneath the image so the
  cursor is still visible around the image padding.
- **Entering edit mode inside a table or mermaid block lands on the
  correct source line.** `source_line_at` previously returned only the
  block's opening line, so `i` from the middle of a 20-row table dropped
  you on the header. Tables now track per-row source lines via a new
  `TableBlock::row_source_lines` vector populated from
  pulldown-cmark's `OffsetIter`. Mermaid blocks interpolate as
  `fence + 1 + K`, clamped to the content length — same pattern code
  blocks already use for their content rows.

### Added
- **Cursor position in the viewer status bar.** The status bar now
  shows `(cursor_line / total_lines, percentage)` instead of the old
  scroll-based percentage, so `d`/`u`/`gg`/`G`/`PageDown`/`PageUp`
  navigation is reflected immediately even when the cursor stays
  on-screen.

## [1.2.0] - 2026-04-15

### Added
- **Visible viewer cursor.** The viewer now shows a highlighted cursor row
  (background from `palette.selection_bg`, carries through line wrapping)
  that moves with `j`/`k`/`d`/`u`/`PageDown`/`PageUp`/`gg`/`G`. Scroll
  follows the cursor when it would leave the viewport, so the observable
  behaviour of "press `j` to scroll down" is preserved while unlocking a
  proper notion of "where I am" for future features.
- **Vim-style edit mode** via
  [edtui](https://crates.io/crates/edtui) 0.11.2. Press `i` in the viewer
  to drop into a modal editor at the exact source line of the viewer
  cursor. Normal/Insert/Visual modes with vim motions (`w`, `b`, `e`,
  `gg`, `G`, `0`, `$`, `dd`, `yy`, `p`, etc.). `:w` saves atomically
  (tempfile + rename), `:q` returns to the rendered view, `:wq` does
  both, `:q!` force-discards unsaved changes. Undo/redo via `u` /
  `Ctrl+r`. The editor theme tracks the active UI palette.
- **Source-line plumbing through the renderer.** pulldown-cmark byte
  offsets are now threaded through `MdRenderer` so every rendered logical
  line reports its originating source line. `DocBlock::Text` carries a
  parallel `source_lines: Vec<u32>`; `DocBlock::Mermaid` and `TableBlock`
  carry `source_line: u32`. This is what powers exact cursor-to-editor
  positioning and unlocks future line-aware features.
- **Git status refresh on save.** Editing a file and pressing `:w` now
  recolors its entry in the file tree immediately — new files turn
  yellow (modified) as soon as the write lands, no git poll wait.

### Changed
- `j`/`k`/`d`/`u`/`PageDown`/`PageUp`/`gg`/`G` in the viewer now move a
  cursor rather than the scroll offset directly. Scroll follows cursor,
  so the visible effect is the same — but the cursor is the new primary
  concept for "where I am".
- `edtui` is pulled in with `default-features = false` to avoid the
  `arboard` clipboard dependency. Our app handles mouse and clipboard
  separately, and this keeps the binary smaller and headless-safe.

### Fixed
- Mouse events are now ignored while `Focus::Editor` is active, so clicks
  in the tree panel during editing no longer select and open files.

## [1.1.0] - 2026-04-14

### Added
- **Syntax highlighting for fenced code blocks.** Fenced blocks with a
  language tag (`rust`, `python`, `javascript`, `go`, `json`, `bash`, and
  many more) are now tokenised and colored inline. Implemented via
  [syntect](https://crates.io/crates/syntect) with the pure-Rust
  `default-fancy` feature — no C dependencies, no onig. Each UI theme
  maps to a bundled syntect theme so colors track the active palette.
- **Table modal mouse support.** The full-screen table viewer (`Enter`
  on a table) now responds to the mouse wheel: plain scroll pans rows,
  `Shift`+scroll pans columns, and clicking outside the modal closes it.
- **Column-boundary horizontal panning in the table modal.** `h` and `l`
  now snap to the previous/next column boundary rather than moving a
  single cell at a time. `H` and `L` pan half a page instead of a fixed
  ten cells, making wide tables dramatically faster to navigate.
- **`scroll_left` / `scroll_right` (`MouseEventKind::ScrollLeft` /
  `ScrollRight`)** are handled where terminals emit them, mapping to
  one-column-boundary pans.

### Fixed
- **Code block right-border alignment.** Lines containing multi-byte
  characters (em dashes, CJK, emoji) no longer push the box frame out of
  alignment. Width measurement now uses `unicode-width` display cells
  throughout instead of byte length.

### Changed
- `render_markdown` and `MarkdownViewState::load` now take the active
  `Theme` so fenced code blocks can be highlighted with a matching
  syntect theme. Callers inside the crate are updated accordingly.

[1.1.0]: https://github.com/leboiko/markdown-reader/releases/tag/v1.1.0
