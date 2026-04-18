# mermaid-text

Render [Mermaid](https://mermaid.js.org/) `graph`/`flowchart` diagrams as
Unicode box-drawing text — no browser, no image protocol, pure Rust.

<!-- badge placeholder: ![crates.io](https://img.shields.io/crates/v/mermaid-text) -->
<!-- badge placeholder: ![docs.rs](https://img.shields.io/docsrs/mermaid-text) -->

## Demo

**Input**

```
graph LR; A[Build] --> B[Test] --> C[Deploy]
```

**Output**

```
┌───────┐      ┌──────┐      ┌────────┐
│ Build │─────▸│ Test │─────▸│ Deploy │
└───────┘      └──────┘      └────────┘
```

---

## Installation

```toml
[dependencies]
mermaid-text = "0.1"
```

or:

```sh
cargo add mermaid-text
```

---

## Usage

### Library API

```rust
fn main() {
    let src = "graph LR; A[Build] --> B[Test] --> C[Deploy]";
    let output = mermaid_text::render(src).unwrap();
    println!("{output}");
}
```

For width-constrained output (terminal-friendly):

```rust
let output = mermaid_text::render_with_width(src, Some(80)).unwrap();
```

The output is a plain `String` — deterministic, newline-delimited, no ANSI
escapes.  Agents and pipelines can parse it line-by-line or search for node
labels by substring.

### CLI

Build and run from the crate root:

```sh
# From a file
cargo run -p mermaid-text -- diagram.mmd

# From stdin
echo "graph LR; A-->B-->C" | cargo run -p mermaid-text

# With a column budget
echo "graph LR; A-->B-->C" | cargo run -p mermaid-text -- --width 60
```

After installing (`cargo install mermaid-text`):

```sh
echo "graph LR; A-->B" | mermaid-text
mermaid-text --width 80 my_diagram.mmd
```

---

## Supported Syntax

| Feature | Supported |
|---------|-----------|
| `graph`/`flowchart` keyword | yes |
| Directions: `LR`, `TD`/`TB`, `RL`, `BT` | yes |
| Rectangle `A[text]` | yes |
| Rounded `A(text)` | yes |
| Diamond `A{text}` | yes |
| Circle `A((text))` | yes |
| Stadium `A([text])` | yes |
| Subroutine `A[[text]]` | yes |
| Cylinder `A[(text)]` | yes |
| Hexagon `A{{text}}` | yes |
| Asymmetric `A>text]` | yes |
| Parallelogram `A[/text/]` | yes |
| Trapezoid `A[/text\]` | yes |
| Double circle `A(((text)))` | yes |
| Solid arrow `-->` | yes |
| Plain line `---` | yes |
| Dotted arrow `-.->` | yes |
| Thick arrow `==>` | yes |
| Bidirectional `<-->` | yes |
| Circle endpoint `--o` | yes |
| Cross endpoint `--x` | yes |
| Edge labels `\|label\|` and `-- label -->` | yes |
| Subgraphs (`subgraph … end`) | yes |
| Nested subgraphs | yes |
| Per-subgraph `direction` override | partial |
| `style`, `classDef`, `linkStyle` | silently ignored |
| `sequenceDiagram`, `pie`, `gantt`, etc. | not supported |

---

## Examples

### State machine

```
graph TD
    Idle -->|event| Running
    Running -->|done| Done
    Running -->|error| Failed
    Failed -->|retry| Idle
```

```
       ┌──────┐
       │ Idle │
       └──────┘
          │ event
          ▾
       ┌─────────┐       ┌────────┐
       │ Running │──────▸│  Done  │
       └─────────┘  done └────────┘
          │ error
          ▾
       ┌────────┐
       │ Failed │
       └────────┘
          │ retry
          ▾
       ┌──────┐
       │ Idle │
       └──────┘
```

### Supervisor pattern

```
graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]
```

### CI/CD pipeline with edge styles

```
graph LR
    subgraph CI
        L[Lint] ==> B[Build] ==> T[Test]
    end
    T ==>|pass| D[Deploy]
    T -.->|skip| D
```

(Thick `==>` = critical path; dotted `-.->` = optional path.)

### Dependency graph

```
graph LR
    App --> DB[(PostgreSQL)]
    App --> Cache[(Redis)]
    App --> Queue[(RabbitMQ)]
    Queue --> Worker[Worker]
    Worker --> DB
```

---

## How It Works

**Parse** — The hand-rolled parser (`parser/flowchart.rs`) splits the input
on newlines and semicolons, identifies node definitions, edge chains, and
subgraph blocks, and builds a `Graph` struct containing typed `Node`, `Edge`,
and `Subgraph` values. Edge style (solid/dotted/thick) and endpoint type
(arrow/circle/cross/none) are parsed and stored.

**Layer + order** — The layered layout (`layout/layered.rs`) assigns each node
to a layer via longest-path from sources, then runs an iterative barycenter
heuristic (up to 8 passes with early termination after 4 non-improving passes,
keeping the best-seen ordering) to minimise edge crossings within each layer.
Per-subgraph `direction` overrides are handled by collapsing the subgraph's
nodes to a single parent layer and then re-running longest-path for downstream
nodes.

**Render** — A 2D character grid (`layout/grid.rs`) stores one `char` per cell
plus a parallel 4-bit direction-mask layer. Each cell's direction mask encodes
which sides have an outgoing line segment; a lookup table converts the mask to
the correct box-drawing glyph (`┼`, `├`, `┤`, `┬`, `┴`, `─`, `│`, etc.)
automatically. Edges are routed via A* pathfinding that treats node bounding
boxes as hard obstacles and already-drawn edges as soft obstacles. After
routing, styled (thick/dotted) edges overwrite the solid glyphs; endpoint
markers (circle, cross, bidirectional arrows) are placed last.

---

## Performance Notes

The library is synchronous and depends only on `unicode-width`. The A*
router has O(W×H) memory for the grid and O(E × W×H log(W×H)) time in the
worst case, where W and H are the grid dimensions. In practice, graphs of
up to ~100 nodes render in well under 10 ms on modern hardware. Very dense
graphs (hundreds of nodes or edges) may produce large grids that increase
render time proportionally.

---

## Limitations

- **Dotted junctions render as solid** — Unicode has no dotted T-junction or
  cross glyphs, so dotted segments revert to solid box-drawing characters at
  intersections.
- **RL/BT subgraph direction does not reverse internal order** — nodes inside
  a RL/BT subgraph are laid out in forward order.
- **Deeply-nested alternating `direction` overrides** — only the top-level
  graph direction is used when evaluating whether a subgraph is orthogonal.
  LR-inside-TB-inside-LR collapses the inner LR band but does not propagate
  the fix outward.
- **Long labels in narrow columns** — compaction reduces gaps but cannot
  reflow labels; extremely narrow `max_width` values may produce overlapping
  node boxes.

---

## Contributing

1. Run `cargo test -p mermaid-text --all-targets` — all tests must pass.
2. Run `cargo clippy -p mermaid-text --all-targets -- -D warnings` — no warnings.
3. Run `cargo doc -p mermaid-text --no-deps` — no doc warnings.
4. Add or update tests for any behavioural change.
5. Update this README and `CHANGELOG.md` if the change is user-visible.

---

## License

MIT

---

## Acknowledgements

Rendering techniques — including the direction-bit canvas, barycenter
heuristic constants, and subgraph border padding — were adapted from
[termaid](https://github.com/fasouto/termaid), a Python prior art library
for rendering Mermaid diagrams as terminal text.
