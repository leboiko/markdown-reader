# Mermaid Diagram Gallery

Every Mermaid feature `markdown-reader` (via the
[`mermaid-text`](https://crates.io/crates/mermaid-text) crate) can render
in your terminal, with one runnable example per feature.

Open this file in `markdown-reader` to see the diagrams rendered live.
You can also paste any code block into a `.mmd` file and run
`mermaid-text path/to/file.mmd` to see the same output on the command
line.

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

## Not yet supported

These Mermaid diagram types fall back to showing source text rather
than rendering:

- `gantt`
- `journey`
- `erDiagram`
- `classDiagram`
- `rect …` colour-highlight blocks inside sequence diagrams (the
  block grammar itself is supported — only the colour form is
  deferred)
- Slice colours in pie charts (rendered monochrome in v1)

See the [ROADMAP](../ROADMAP.md) for what's planned next, and file
issues at <https://github.com/leboiko/markdown-reader/issues> if you
hit something specific.
