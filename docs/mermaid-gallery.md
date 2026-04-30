# Mermaid Diagram Gallery

Every Mermaid feature `markdown-reader` (via the
[`mermaid-text`](https://crates.io/crates/mermaid-text) crate) can render
in your terminal, with one runnable example per feature.

Open this file in `markdown-reader` to see the diagrams rendered live.
You can also paste any code block into a `.mmd` file and run
`mermaid-text path/to/file.mmd` to see the same output on the command
line.

## What's covered

- **Flowcharts** (`graph` / `flowchart`) with subgraphs, edge styles,
  `classDef` colours, and long-edge waypoint routing for clean channels
  through complex graphs.
- **State diagrams** with composite (nested) states, `<<choice>>` /
  `<<fork>>` / `<<join>>` shape modifiers, anchored notes, and
  `classDef` colour styling.
- **Sequence diagrams** — feature-complete: `autonumber`, notes
  (single + multi-anchor + `<br>` line breaks), activation bars
  (explicit + inline `+`/`-`), block statements (`loop`/`alt`/`opt`/
  `par`/`critical`/`break` with arbitrary nesting), and bracketed
  lifelines (boxes top AND bottom).
- **Entity-relationship diagrams** (`erDiagram`) with attribute
  tables inside each entity box, single-character cardinality glyphs
  at endpoints (`1`, `?`, `+`, `*`), and identifying vs
  non-identifying line styles.
- **Pie charts** rendered as horizontal bar charts (more legible in
  monospace than any ASCII pie attempt).
- **Gantt charts** (`gantt`) rendered as Unicode horizontal bar charts
  with a tick-labelled date axis, section headings, `█`/`░` task bars,
  and `[start → end, Nd]` annotations. Supports explicit dates, `after
  <id>` dependencies, and chained implicit-start tasks. Phase 1
  limitations: status tags (`done`, `active`, `crit`, `milestone`) and
  `excludes`/`includes` are silently ignored.
- **Timeline diagrams** (`timeline`) rendered as a vertical
  bullet-on-a-wire flow. Each section has a `── Name ─────` header;
  each time period gets a `●──` bullet; additional events for the same
  period hang below with `└──` connectors. Phase 1 limitations: `&`
  relationship links and custom colour themes are silently ignored.
- **Git graph diagrams** (`gitGraph`) rendered as a lane-based commit
  graph with one branch per vertical column and time flowing
  top-to-bottom. Glyphs: `*` normal commit, `M` merge commit, `C`
  cherry-pick; `╭╮╰╯─` for fork and merge arcs; `│` for lane
  continuation. Commit ids and optional `[tag]` annotations appear to
  the right. Branch names are printed at the bottom of each lane.
  Phase 1 limitations: direction modifiers (`LR`/`TB`), extended
  commit types (`REVERSE`/`HIGHLIGHT`), and custom themes are silently
  ignored.
- **Mindmap diagrams** (`mindmap`) rendered as a vertical Unicode tree.
  The root node is displayed in a `╭─…─╮` rounded box at the top with a
  trunk `│` connector leading to child nodes. Non-last children use
  `├──`; last children use `└──`; continuation pipes `│   ` track
  open branches at each nesting level. Phase 1 limitations: all 6
  Mermaid node shapes (default, rounded, circle, bang, cloud, hexagon)
  are normalised to plain text; `::icon(...)` directives are silently
  ignored; custom colour themes have no effect.
- **Quadrant chart diagrams** (`quadrantChart`) rendered as a 2x2
  priority matrix. A horizontal axis (drawn with `─` and `┼`) and a
  vertical axis (drawn with `│`, `^`, `v`) divide the canvas into four
  quadrants. Quadrant labels appear in the four corners (Q1 top-right,
  Q2 top-left, Q3 bottom-left, Q4 bottom-right). Data points are placed
  proportionally using `·` markers followed by the point name and
  coordinates. Axis edge labels sit on the outermost ends of each axis.
  Phase 1 limitations: custom point styling (colour, radius) and
  background quadrant colours/gradients are not supported; points that
  map to the same terminal cell overlap (last in source order wins).
- **Requirement diagrams** (`requirementDiagram`) rendered as labeled
  boxes with a relationship summary. Requirements use straight-cornered
  boxes (`┌┐└┘`) with a `<<kind>>` stereotype header row and a
  key-value data table (`id`, `text`, `risk`, `verifymethod`). Elements
  use rounded-cornered boxes (`╭╮╰╯`) to visually distinguish them from
  requirements. Relationships are listed as `source --[kind]--> target`
  lines below all boxes. Phase 1 limitations: layout is purely vertical
  (no side-by-side arrangement); relationship arcs are a text summary,
  not graphical lines; custom styling is not supported.
- **XY charts** (`xychart-beta` / `xychart`) rendered as a Unicode bar/line
  chart. Bar series use `█` block columns; line series plot `●` point markers
  connected with `╭─╯╰│` curve glyphs. Both series can coexist (bars drawn
  first, line overlaid). The y-axis shows right-aligned numeric tick labels;
  the x-axis shows category labels or range endpoints below a `└─┬─` connector
  row. Respects `max_width` via proportional column sizing. Phase 1 limitations:
  only the last `bar` / `line` definition is kept; horizontal orientation is
  parsed but rendered vertically; no custom colours.
- **Block diagrams** (`block-beta` / `block`) rendered as a fixed-width grid
  of Unicode rectangle boxes. `columns N` sets the grid column count; `id:N`
  spans a block across N columns (the combined column widths and gaps become
  the box width); `id["text"]` sets the display label (otherwise the id is
  used). Directed edges (`A --> B`) are listed below the grid as
  `src ──► target` text lines. Respects `max_width` via iterative
  column-width reduction. Phase 1 limitations: all block shapes are normalised
  to plain rectangles; nested `block … end` blocks are silently skipped;
  vertical spans are not supported; edge labels appear in the text summary only.
- **Packet diagrams** (`packet-beta` / `packet`) rendered as a 32-bit-wide
  row table. Each 32-bit word is one row; fields occupy their proportional bit
  columns. A bit-number ruler is printed above each row. Field labels are
  centred in their cells; labels that are too wide are truncated with `…`.
  Phase 1 limitations: row width is fixed at 32 bits; no custom colours;
  `accTitle`/`accDescr` silently ignored.
- **Sankey diagrams** (`sankey-beta` / `sankey`) rendered as a grouped-flow
  list. Each source node is a header; outgoing flows appear below with a
  proportional Unicode bar (`█` full-block + `▏▎▍▌▋▊▉` sub-cell eighths) so
  magnitudes are visually comparable. A single global scale factor spans all
  flows in the diagram. Phase 1 limitations: no curvilinear bands or Sugiyama
  layout; colours deferred; node heights are not scaled.
- **Architecture diagrams** (`architecture-beta` / `architecture`) rendered
  as labeled group border boxes containing horizontal rows of service boxes.
  Top-level services appear as standalone boxes above the group section.
  Connections between services are listed below as a text summary preserving
  port specifiers (`db:L ─── R:server`). Phase 1 limitations: icon names are
  stored but not rendered; junction nodes silently skipped; no spatial edge
  routing; all services in a group appear in one horizontal row.

Recent rendering improvements: arrow tips merge into destination box
borders (`┌─▾─┐` instead of floating `▾` above), edge labels never
puncture node corners or subgraph borders, and block-statement frames
use a Mermaid-style two-tag style (`╔═[alt]══[cache hit]══╗`).

---

## Flowcharts

### Basic flowchart with directions

```mermaid
flowchart LR
    A[Start] --> B{Decision}
    B -->|yes| C[Build]
    B -->|no| D[Skip]
    C --> E[Deploy]
    D --> E
```

Directions are `LR` (left→right), `RL`, `TB` (top→bottom), and `BT`.

### Subgraphs

```mermaid
flowchart TB
    subgraph frontend [Frontend]
        UI[Browser UI]
        SW[Service Worker]
    end
    subgraph backend [Backend]
        API[REST API]
        DB[(Postgres)]
    end
    UI --> API
    SW --> API
    API --> DB
```

### Edge styles and labels

```mermaid
flowchart LR
    A --> B
    A -.-> C
    A ==> D
    A -- "labelled" --> E
    A -. "dashed label" .-> F
    A == "thick label" ==> G
```

### Node shape showcase

All 13 supported node shapes in one diagram. Each shape has a distinct
visual treatment so they can be told apart at a glance:

```mermaid
graph LR
    A[Square]
    B(Round)
    C((Circle))
    D{Rhombus}
    E[[Subroutine]]
    F[(Database)]
    G{{Hexagon}}
    H[/Parallelogram/]
    I[\BackSlash\]
    J[/Trapezoid\]
    K[\InvTrapezoid/]
    L([Stadium])
    M>Asymmetric]
```

Expected terminal output (0.25.0+):

```
┌────────┐     ╭───────╮     ╭──────────╮     ╱─────────╲
│ Square │     │ Round │     (  Circle  )     │ Rhombus │
└────────┘     ╰───────╯     ╰──────────╯     ╲─────────╱

┌──────────────┐     ╭──────────╮     ╱───────────╲
││ Subroutine ││     │ ──────── │     <  Hexagon  >
└──────────────┘     │ Database │     ╲───────────╱
                     ╰──────────╯

╱─────────────────╱     ╲─────────────╲     ╱─────────────╲
│  Parallelogram  │     │  BackSlash  │     │  Trapezoid  │
╱─────────────────╱     ╲─────────────╲     └─────────────┘

╲────────────────╱     ╭───────────╮     ┌──────────────┐
│  InvTrapezoid  │     (  Stadium  )     │  Asymmetric  ⟩
└────────────────┘     ╰───────────╯     └──────────────┘
```

Key shape distinctions (updated in 0.25.0):

- **Diamond** `{label}` — `╱` top-left / `╲` top-right corners, `╲` / `╱` bottom.
  Clearly distinguishes a decision node from a plain rectangle.
- **Circle** `((label))` — `(` / `)` replace the side border at the midpoint row.
  The label text is undecorated ("Circle", not "( Circle )").
- **Stadium** `([label])` — same `(` / `)` border-overwrite trick as Circle.
  The parens sit ON the border, not inside the text area.
- **Cylinder** `[(label)]` — rounded box with an interior `─` lip line below the
  top border. Suggests a barrel/database cap without a misleading divider.
- **Hexagon** `{{label}}` — `╱`/`╲` diagonal corners PLUS `<`/`>` side-point markers.
  Six visual edges approximate a true hexagon.
- **Parallelogram** `[/label/]` — `╱` at all four corners (consistent lean-right).
- **BackSlash Parallelogram** `[\label\]` — `╲` at all four corners (lean-left mirror).
- **Trapezoid** `[/label\]` — `╱` top-left, `╲` top-right, square bottom corners.
- **Inverted Trapezoid** `[\label/]` — `╲` top-left, `╱` top-right, square bottom corners.

### Colors via classDef + class

```mermaid
flowchart LR
    classDef hot fill:#f33,stroke:#900,color:#fff
    classDef cold fill:#39f,stroke:#039,color:#fff
    A[input] --> B[transform]:::hot
    B --> C[cache]:::cold
    C --> D[output]
```

`mermaid-text` honours these colours when rendered with `--color`
(24-bit ANSI). The TUI viewer enables it automatically.

---

## State diagrams

### Basic state machine

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Running : start
    Running --> Paused : pause
    Paused --> Running : resume
    Running --> Idle : stop
    Idle --> [*]
```

### Composite states (nested)

```mermaid
stateDiagram-v2
    [*] --> Active
    state Active {
        [*] --> Idle
        Idle --> Working : task
        Working --> Idle : done
    }
    Active --> [*]
```

### Choice / fork / join shape modifiers

```mermaid
stateDiagram-v2
    state route_check <<choice>>
    [*] --> route_check
    route_check --> Cached : hit
    route_check --> Origin : miss
    Cached --> [*]
    Origin --> [*]
```

### Notes anchored to a state

```mermaid
stateDiagram-v2
    [*] --> CircuitOpen
    CircuitOpen --> CircuitClosed : timeout reached
    CircuitClosed --> CircuitOpen : 5 errors
    note right of CircuitOpen
        Open state rejects all
        traffic for cool-down period.
    end note
```

---

## Sequence diagrams

### Minimal call/reply

```mermaid
sequenceDiagram
    Alice->>Bob: hello
    Bob-->>Alice: hi back
```

`->>` = solid arrow with arrowhead, `-->>` = dashed (typical for replies).
Plain `->` and `-->` are no-arrowhead variants.

### Participants and aliases

```mermaid
sequenceDiagram
    participant U as User
    participant API
    participant DB as Postgres
    U->>API: GET /orders
    API->>DB: SELECT * FROM orders
    DB-->>API: rows
    API-->>U: 200 OK
```

### Autonumber

```mermaid
sequenceDiagram
    autonumber
    Client->>Server: POST /login
    Server->>Auth: verify(token)
    Auth-->>Server: ok
    Server-->>Client: 200 + session
```

`autonumber 100` re-bases the counter; `autonumber off` halts numbering
mid-diagram.

### Notes (single anchor and spanning)

```mermaid
sequenceDiagram
    participant U as User
    participant API
    note over U,API : Authentication flow
    U->>API: POST /login
    API->>U: 200 + token
    note right of U : token cached for 1h
```

`note left of X`, `note right of X`, `note over X`, and
`note over X,Y` (spanning two participants) are supported. Use `<br>` or
`<br/>` for multi-line note text.

### Activation bars

```mermaid
sequenceDiagram
    participant U as User
    participant API
    participant DB
    U->>+API: POST /login
    API->>+DB: SELECT user
    DB-->>-API: user record
    API-->>-U: 200 + token
```

`+` on the message target activates the receiver; `-` deactivates the
sender. Explicit `activate X` / `deactivate X` directives also work,
including arbitrary nesting on the same participant.

### Block statements

```mermaid
sequenceDiagram
    participant U as User
    participant API
    loop daily
        U->>API: POST /login
        alt cache hit
            API->>U: cached token
        else cache miss
            API->>U: fresh token
        end
    end
```

Supported blocks: `loop`, `alt`/`else`, `opt`, `par`/`and`,
`critical`/`option`, `break`. Nested blocks inset by one cell per
nesting level so they read distinctly.

### Everything together

```mermaid
sequenceDiagram
    autonumber
    participant U as User
    participant API
    participant DB
    note over U,API : Authentication flow
    loop daily
        U->>+API: POST /login
        alt cache hit
            API->>U: cached token
        else cache miss
            API->>+DB: SELECT user
            DB-->>-API: user record
            API-->>-U: 200 + token
        end
    end
```

All four sequence-polish features compose: autonumber + notes +
activation bars + block statements in a single diagram.

---

## Entity-relationship diagrams

### Canonical customer/order schema

```mermaid
erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ LINE-ITEM : contains
    CUSTOMER {
        string name
        string email PK
    }
    ORDER {
        int    orderNumber   PK
        date   orderDate
        string customerEmail FK
    }
    LINE-ITEM {
        string productCode
        int    quantity
        float  unitPrice
    }
```

Entities render as boxes with attribute tables (type / name / keys
columns). Relationships carry single-character cardinality glyphs at
each endpoint:

- `1` — exactly one
- `?` — zero or one
- `+` — one or many
- `*` — zero or many

The connector is solid (`─`) for identifying relationships (`--` in
Mermaid) and dashed (`┄`) for non-identifying (`..`).

### Optional (non-identifying) relationship

```mermaid
erDiagram
    PARENT ||..o{ CHILD : optional
    PARENT {
        int id PK
    }
    CHILD {
        int id PK
        int parentId FK
    }
```

The `||..o{` connector renders as a dashed line to mark it as
non-identifying — the child could exist independently of the parent.

### Wide schema: grid layout (Phase 3)

When a diagram has more than ~5 entities (or when `max_width` is set and the
single-row layout would exceed it), the renderer wraps entities into a
`ceil(sqrt(n))`-column grid. Relationships between entities in the same row
use the existing horizontal routing; cross-row relationships route via a
vertical spine on the right margin of the canvas.

```mermaid
erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ ITEM : contains
    PRODUCT ||--o{ ITEM : describes
    CATEGORY ||--o{ PRODUCT : groups
    ACCOUNT ||--|| CUSTOMER : owns
    INVOICE ||--|{ ORDER : bills
    CUSTOMER { int id PK  string name }
    ORDER    { int id PK  int customerId FK }
    PRODUCT  { int id PK  string name  int categoryId FK }
    CATEGORY { int id PK  string label }
    ACCOUNT  { int id PK }
    INVOICE  { int id PK }
    ITEM     { int orderId FK  int productId FK }
```

With a 40-column budget, the 7 entities above render in a 3-column grid
(ceil(sqrt(7)) = 3) with cross-row arrows routed along the right spine.
Small diagrams (≤ 5 entities, or those that fit within the budget) are
unaffected — they continue to render in a single row.

---

## Pie charts

### Basic pie

```mermaid
pie title Pet Counts
    "Dogs" : 386
    "Cats" : 85
    "Rats" : 15
```

Renders as a horizontal bar chart in monospace text — far more legible
than any ASCII pie attempt.

### With raw values (`showData`)

```mermaid
pie showData title Browser Market Share
    "Chrome" : 64.7
    "Safari" : 18.7
    "Edge" : 5.4
    "Firefox" : 3.4
    "Opera" : 2.5
    "Other" : 5.3
```

The `showData` keyword adds a `(value)` column next to each slice's
percentage. Decimal values are supported.

---

---

## Gantt charts

Gantt diagrams render as horizontal bar charts, one task per row. Task bars
(`█` active, `░` empty) are scaled to fit the terminal width. A tick-labelled
date axis appears above the bars.

### Simple project schedule

```mermaid
gantt
    title Software Release v2
    dateFormat YYYY-MM-DD
    axisFormat %m-%d
    section Design
      Research       :r1, 2024-01-01, 7d
      Wireframes     :after r1, 5d
    section Development
      Backend        :b1, 2024-01-13, 14d
      Frontend       :after b1, 10d
    section QA
      Testing        :2024-02-06, 7d
```

Expected output (trimmed to 80 columns):

```text
Gantt: Software Release v2 (2024-01-01 → 2024-02-12, 43 days)

                  01-01    01-08    01-15    01-22    01-29    02-05
Design
  Research        ░███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  [01-01 → 01-07, 7d]
  Wireframes      ░░░░░░░░░░█████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  [01-08 → 01-12, 5d]

Development
  Backend         ░░░░░░░░░░░░░░░░█████████████████░░░░░░░░░░░  [01-13 → 01-26, 14d]
  Frontend        ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░███████░░  [01-27 → 02-05, 10d]

QA
  Testing         ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░███░  [02-06 → 02-12, 7d]
```

### Classic Mermaid example with axisFormat %b %d

```mermaid
gantt
    title A Gantt Diagram
    dateFormat YYYY-MM-DD
    axisFormat %b %d
    section Section A
        Design        :a1, 2014-01-01, 30d
        Implementation:after a1, 20d
    section Section B
        Testing       :2014-02-15, 15d
        Deployment    :3d
```

Expected output (trimmed to 80 columns):

```text
Gantt: A Gantt Diagram (2014-01-01 → 2014-03-04, 63 days)

                  Jan 01   Jan 11    Jan 21   Jan 31    Feb 10   Feb 20    Mar

Section A
  Design          ████████████████████████████░░░░░░░░░░░░░░░░░░░░  [Jan 01 → Jan 30, 30d]
  Implementation  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████████████████░  [Jan 31 → Feb 19, 20d]

Section B
  Testing         ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████  [Feb 15 → Mar 01, 15d]
  Deployment      ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██  [Mar 02 → Mar 04, 3d]
```

**Phase 1 limitations.** Status tags (`done`, `active`, `crit`, `milestone`)
are silently ignored. `excludes`/`includes` (weekend skipping) and
`tickInterval` are not implemented. Non-`YYYY-MM-DD` date formats produce a
parse error. See `mermaid-text` 0.20.0 CHANGELOG for the full list.

---

## Timeline diagrams

### Social media history (two sections, multi-event period)

```mermaid
timeline
    title History of Social Media
    section 2002-2004
        2002 : LinkedIn
        2003 : MySpace launched
        2004 : Facebook : Google goes public
    section 2005-2008
        2005 : YouTube
        2006 : Twitter
        2007 : iPhone : Tumblr
```

Expected output:

```text
Timeline: History of Social Media

── 2002-2004 ─────────────────────────────────────
  2002 ●── LinkedIn
  2003 ●── MySpace launched
  2004 ●── Facebook
       └── Google goes public

── 2005-2008 ─────────────────────────────────────
  2005 ●── YouTube
  2006 ●── Twitter
  2007 ●── iPhone
       └── Tumblr
```

### Technology milestones (implicit unnamed section)

Events that appear before any `section` keyword land in an implicit unnamed
section — no section header is rendered for those entries.

```mermaid
timeline
    title Key Technology Milestones
    1991 : World Wide Web
    1993 : Mosaic browser
    section Late 90s
        1995 : JavaScript : Java applets
        1998 : Google founded
    section 2000s
        2001 : Wikipedia
        2004 : Gmail
        2007 : iPhone
```

Expected output (the 1991–1993 events appear without a section header):

```text
Timeline: Key Technology Milestones

  1991 ●── World Wide Web
  1993 ●── Mosaic browser

── Late 90s ─────────────────────────────────────
  1995 ●── JavaScript
       └── Java applets
  1998 ●── Google founded

── 2000s ────────────────────────────────────────
  2001 ●── Wikipedia
  2004 ●── Gmail
  2007 ●── iPhone
```

**Phase 1 limitations.** `&` relationship links between events are treated
as literal text. Custom colour themes are silently ignored.

---

## Git graph diagrams

A `gitGraph` diagram shows a commit history across one or more branches as a
lane-based text diagram. Time flows top-to-bottom; each branch occupies a
vertical column.

### Example 1 — main branch with a feature branch and merge

```mermaid
gitGraph
    commit
    commit id: "second"
    branch develop
    checkout develop
    commit
    commit id: "feature-x"
    checkout main
    merge develop
    commit tag: "v1.0"
```

Expected text-mode output:

```
*    c0
*    second
╭ ╮
│ *  c2
│ *  feature-x
╰ ╯
M │  c4
* │  c5 [v1.0]
main develop
```

### Example 2 — cherry-pick commit

```mermaid
gitGraph
    commit id: "init"
    branch hotfix
    checkout hotfix
    commit id: "patch"
    checkout main
    cherry-pick id: "patch"
    commit tag: "v1.1"
```

Expected text-mode output:

```
*    init
╭ ╮
│ *  patch
╰ ╯
C │  c3
* │  c4 [v1.1]
main hotfix
```

**Phase 1 limitations.** `gitGraph LR` direction modifiers are silently
ignored. Extended commit types (`REVERSE`, `HIGHLIGHT`) and `commit message:`
are silently ignored. Custom themes have no effect. Branch lanes are ordered
strictly by creation order with no crossing minimisation.

---

## Mindmap diagrams

A `mindmap` diagram is an indent-based hierarchical outline. The first node
after the `mindmap` keyword is the root; deeper indentation creates children.
The root is rendered in a rounded box at the top; the rest of the tree hangs
below it using standard tree-drawing connectors.

**Example 1 — Canonical Mermaid mindmap:**

```mermaid
mindmap
  root((mindmap))
    Origins
      Long history
      ::icon(fa fa-book)
      Popularisation
        British popular psychology author Tony Buzan
    Research
      On effectiveness and features
      On Automatic creation
        Uses
          Creative techniques
          Strategic planning
          Argument mapping
    Tools
      Pen and paper
      Mermaid
```

Expected rendered output:

```text
╭─────────╮
│ mindmap │
╰────┬────╯
     │
├── Origins
│   ├── Long history
│   └── Popularisation
│       └── British popular psychology author Tony Buzan
├── Research
│   ├── On effectiveness and features
│   └── On Automatic creation
│       └── Uses
│           ├── Creative techniques
│           ├── Strategic planning
│           └── Argument mapping
└── Tools
    ├── Pen and paper
    └── Mermaid
```

**Example 2 — Software architecture decision:**

```mermaid
mindmap
  root((Decision))
    Option A
      Pros
        Fast to implement
        Low cost
      Cons
        Hard to scale
    Option B
      Pros
        Highly scalable
        Well documented
      Cons
        Higher upfront effort
    Recommendation
      Option B for long-term growth
```

Expected rendered output:

```text
╭──────────╮
│ Decision │
╰─────┬────╯
      │
├── Option A
│   ├── Pros
│   │   ├── Fast to implement
│   │   └── Low cost
│   └── Cons
│       └── Hard to scale
├── Option B
│   ├── Pros
│   │   ├── Highly scalable
│   │   └── Well documented
│   └── Cons
│       └── Higher upfront effort
└── Recommendation
    └── Option B for long-term growth
```

**Phase 1 limitations.** All 6 Mermaid node shapes (`((circle))`, `(rounded)`,
`{{hexagon}}`, `))bang((`, `)cloud(`, `[rectangle]`) are stripped to their inner
text — the rendered tree does not visually distinguish shapes. `::icon(...)` icon
directives are silently ignored. Custom colour themes have no effect.

---

## Quadrant chart diagrams

A `quadrantChart` diagram plots named data points on a 2x2 priority matrix.
Axis labels describe the low and high ends of each dimension. Quadrant labels
appear in each corner, numbered Q1 (top-right) through Q4 (bottom-right)
counter-clockwise.

**Example 1 — Canonical Mermaid example: campaign reach and engagement:**

```
quadrantChart
    title Reach and engagement of campaigns
    x-axis Low Reach --> High Reach
    y-axis Low Engagement --> High Engagement
    quadrant-1 We should expand
    quadrant-2 Need to promote
    quadrant-3 Re-evaluate
    quadrant-4 May be improved
    Campaign A: [0.3, 0.6]
    Campaign B: [0.45, 0.23]
    Campaign C: [0.57, 0.69]
    Campaign D: [0.78, 0.34]
    Campaign E: [0.40, 0.34]
    Campaign F: [0.35, 0.78]
```

Expected rendered output (approximate; exact column positions depend on width):

```text
                   Reach and engagement of campaigns

                            High Engagement
                                   ^
                  Need to promote  │ We should expand
                                   │
                       · Campaign F (0.35,0.78)
                    · Campaign A (0.30,0.60)  │  · Campaign C (0.57,0.69)
                                   │
Low Reach──────────────────────────┼────────────────────────High Reach
                                   │
                           · Campaign E (0.40,0.34)  · Campaign D (0.78,0.34)
                              · Campaign B (0.45,0.23)
                      Re-evaluate  │ May be improved
                                   v
                            Low Engagement
```

**Example 2 — Minimal chart with just two quadrant labels and one point:**

```
quadrantChart
    x-axis Low Priority --> High Priority
    y-axis Low Impact --> High Impact
    quadrant-1 Quick wins
    quadrant-3 Avoid
    Task A: [0.7, 0.8]
```

Expected rendered output (approximate):

```text
                            High Impact
                                   ^
                                   │  Quick wins
                                   │
                                   │     · Task A (0.70,0.80)
                                   │
Low Priority───────────────────────┼────────────────────────High Priority
                                   │
                  Avoid            │
                                   │
                                   v
                             Low Impact
```

**Phase 1 limitations.** Custom point styling (colour, radius, shape) is not
supported; all points render as a `·` marker. Background quadrant colours and
gradients are not rendered. Points that map to the same terminal cell overlap —
the last point in source order wins. `accTitle` / `accDescr` lines are silently
ignored. Point labels near the right edge may be truncated by the canvas width.

---

## Requirement diagrams

A `requirementDiagram` models formal requirements, real-world elements, and
the relationships between them. Requirements carry an `id`, descriptive `text`,
and optional `risk` and `verifymethod` fields. Elements represent artefacts
such as code modules or documents.

**Example 1 — Minimal requirement + element + relationship:**

```
requirementDiagram

    requirement r1 {
        id: 1
        text: the system shall respond within 200ms.
        risk: high
        verifymethod: test
    }

    element backend {
        type: service
    }

    backend - satisfies -> r1
```

Expected rendered output:

```text
┌────────────────────────────────┐
│        <<requirement>>         │
│               r1               │
├────────────────────────────────┤
│ id:           1                │
│ text:         the system shal… │
│ risk:         high             │
│ verifymethod: test             │
└────────────────────────────────┘

╭────────────────────────────────╮
│            backend             │
├────────────────────────────────┤
│ type: service                  │
╰────────────────────────────────╯

Relationships:
  backend --[satisfies]--> r1
```

**Phase 1 limitations.** Layout is purely vertical — boxes are stacked
top-to-bottom; no side-by-side arrangement is attempted. Relationship arcs are
rendered as a text summary (`--[kind]-->`) rather than graphical lines between
boxes. Custom styling/colours are not supported. `accTitle` / `accDescr` lines
are silently ignored.

---

## XY charts

### Sales revenue (bar + line)

```mermaid
xychart-beta
    title "Sales Revenue"
    x-axis [jan, feb, mar, apr, may, jun, jul, aug, sep, oct, nov, dec]
    y-axis "Revenue (in $)" 4000 --> 11000
    bar [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]
    line [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]
```

**Phase 1 limitations.** Only the last `bar` and the last `line` definition are
kept (multiple series of the same kind are not supported). Horizontal orientation
(`xychart-beta horizontal`) is parsed but rendered vertically. Custom point
styling and colours are not supported. `accTitle` / `accDescr` lines are silently
ignored.

---

## Block diagrams

A `block-beta` diagram renders a fixed-width grid of rectangular blocks with
optional directed edges. `columns N` sets the number of grid columns; blocks
fill left-to-right and wrap to the next row when a row is full. `id:N` makes a
block span N columns; `id["text"]` sets an explicit display label.

Edges between horizontally- or vertically-adjacent blocks are drawn as inline
arrow glyphs (`►` / `◄` / `▼` / `▲`) in the gap between the boxes. Edges
between non-adjacent blocks fall back to a short text summary below the grid.

### Simple 3-column grid with inline arrows

```mermaid
block-beta
    columns 3
    A B C
    A --> B
    B --> C
```

**Expected output (adjacent edges drawn inline in the column gap):**

```
┌───┐ ► ┌───┐ ┌───┐
│ A │   │ B │ │ C │
└───┘   └───┘ └───┘
        ►
┌───┐   ┌───┐ ┌───┐
```

Actually the inline arrow occupies the single-character gap row/column between
boxes:

```
┌───┐ ┌───┐ ┌───┐
│ A │►│ B │►│ C │
└───┘ └───┘ └───┘
```

### Labelled spanning blocks

```mermaid
block-beta
    columns 3
    a["A label"] b:2 c
    d e f
    g["spans across"]:3
    a --> d
    b --> e
    c --> f
```

**Limitations.** All block shapes (rounded, stadium, cylinder, hexagon)
are normalised to plain rectangles. Nested `block … end` blocks are silently
skipped by the parser. Vertical spans (multi-row blocks) are not supported. Edge
labels (`-->|text|`) are parsed but not rendered on the inline arrow. Custom
block colours and `accDescr` / `accTitle` are silently ignored. Non-adjacent
edges (source and target are not immediate neighbours in the grid) fall back to
a text summary below the grid.

---

## Packet diagrams

A `packet-beta` diagram renders a network packet header (or any fixed-width
binary structure) as a 32-bit-wide row table. Each field occupies the columns
proportional to its bit width. A bit-number ruler is printed above each row.

### TCP Packet header

```mermaid
packet-beta
    title TCP Packet
    0-15: "Source Port"
    16-31: "Destination Port"
    32-63: "Sequence Number"
    64-95: "Acknowledgment Number"
    96-99: "Data Offset"
    100-105: "Reserved"
    106: "URG"
    107: "ACK"
    108: "PSH"
    109: "RST"
    110: "SYN"
    111: "FIN"
    112-127: "Window"
    128-143: "Checksum"
    144-159: "Urgent Pointer"
    160-191: "(Options and Padding)"
    192-223: "Data (variable length)"
```

**Expected output:**

```
TCP Packet

 0                                               16                                           31
┌───────────────────────────────────────────────┬───────────────────────────────────────────────┐
│                  Source Port                  │               Destination Port                │
 32                                              48                                           63
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│                                        Sequence Number                                        │
 64                                              80                                           95
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│                                     Acknowledgment Number                                     │
 96                                              112                                          127
├───────────┼─────────────────┼──┼──┼──┼──┼──┼──┼───────────────────────────────────────────────┤
│Data Offset│    Reserved     │U…│A…│P…│R…│S…│F…│                    Window                     │
 128                                             144                                          159
├───────────────────────────────────────────────┼───────────────────────────────────────────────┤
│                   Checksum                    │                Urgent Pointer                 │
 160                                             176                                          191
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│                                     (Options and Padding)                                     │
 192                                             208                                          223
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│                                    Data (variable length)                                     │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Phase 1 limitations.** Row width is fixed at 32 bits (custom widths are not
supported). Single-bit fields display a truncated label with `…` when the label
exceeds the cell width. Multi-row fields (wider than 32 bits) split at the
32-bit boundary; the label appears in the first fragment row only. Custom
colours and `accDescr` / `accTitle` are silently ignored.

---

## Architecture diagrams

An `architecture-beta` diagram describes system components — services grouped
into named clusters — and the connections between them. Services may declare
an icon name (e.g. `cloud`, `database`, `disk`, `server`) which is recorded
but not yet rendered.

Groups map to subgraph containers and services map to nodes in the flowchart
Sugiyama layout engine, so edges are spatially routed with box-drawing lines
around the service boxes rather than listed as a text summary.

**Example 1 — Cloud API cluster:**

```
architecture-beta
    group api(cloud)[API]

    service db(database)[Database] in api
    service disk1(disk)[Storage] in api
    service server(server)[Server] in api

    db:L -- R:server
    disk1:T -- B:server
```

The renderer translates this into the flowchart Sugiyama pipeline: the `api`
group becomes a subgraph container, each service becomes a rectangle node, and
the edges are routed spatially with Unicode line characters.

**Example 2 — Mixed top-level and grouped services:**

```
architecture-beta
    service gateway(internet)[Gateway]

    group backend(cloud)[Backend]
    service api(server)[API] in backend
    service store(database)[Store] in backend

    gateway --> api
    api:R -- L:store
```

Top-level services (not in any group) are rendered as ungrouped nodes outside
the subgraph containers.

**Current limitations.**

- Icon names (`cloud`, `database`, etc.) are parsed and stored but not rendered
  as graphical glyphs — they appear parenthetically in group headers only.
- Junction nodes (`junction(id)`) are silently skipped.
- Port specifiers (`L`/`R`/`T`/`B` on edges) are stored but ignored during
  routing — spatial port-aware attachment is deferred to Path B.

---

## Sankey diagrams

A `sankey-beta` diagram renders directed flow between named nodes. Each source
node appears as a header; its outgoing flows are listed below with a proportional
Unicode bar so you can compare magnitudes at a glance.

### Energy flow (canonical)

```mermaid
sankey-beta

%% source,target,value
Agricultural 'waste',Bio-conversion,124.729
Bio-conversion,Liquid,0.597
Bio-conversion,Solid,280.322
Coal imports,Coal,11.606
Coal,Solid,75.571
```

Terminal output (0.41.0 proportional-bar format):

```text
Agricultural 'waste'  (total: 124.7)
  ████████████████████████████████▌ [124.7] ► Bio-conversion

Bio-conversion  (total: 280.9)
  ▏                                 [  0.6] ► Liquid
  █████████████████████████████████ [280.3] ► Solid

Coal imports  (total: 11.6)
  █                                 [ 11.6] ► Coal

Coal  (total: 75.6)
  █████████                         [ 75.6] ► Solid
```

Bars use full-block `█` glyphs plus sub-cell eighths (`▏▎▍▌▋▊▉`) for
sub-cell precision. A single global scale factor across all flows keeps bars
mutually comparable. Phase 1 limitations: no true Sugiyama-layout curvilinear
bands; colours deferred; node heights not scaled.

---

## Not yet supported

These Mermaid diagram types fall back to showing source text rather
than rendering:

- `rect <colour>` colour-highlight blocks inside sequence diagrams
  (the block grammar itself is supported — only the colour form is
  deferred; Mermaid's grammar is hard to express without bigger
  changes to our colour system).
- Slice colours in pie charts (rendered monochrome in v1; the bar
  chart approach is so legible in monospace that colours haven't
  been requested yet).

File issues at <https://github.com/leboiko/markdown-reader/issues>
if you hit something specific. Quality bugs (rendering glitches,
edge-case crashes) get the fastest turnaround.
