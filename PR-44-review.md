# PR #44 review — delimiter-aware Mermaid labels

Reviewed: 2026-08-31  
Base: `master` at `7763e25`  
Head reviewed: `c1a5617`

## Verdict

Approve after the repaired head passes the full local and remote gates. Every
review finding now has a failing-first regression and a bounded fix.

The original production ceiling was met at 119 net lines. Two semantic quote-
removal cases are duplicated across four baseline files; the evidence and the
decision not to weaken coverage are recorded in `ISSUE-40-SCOPE.md`.

## Findings

| Severity | Finding | Resolution |
|---|---|---|
| Major | `tokenise_chain` and grouped-node splitting treat delimiters and `&` inside balanced quoted shaped labels as syntax. `a["left [ bracket"] --> b[End]` loses its edge; `a["x ] & y"] --> b` creates ghost grouped nodes. | Quote state is scoped to the shaped operand. Exact graph and render tests cover both bracket and grouped-node cases. |
| Major | Statement state leaks across physical boundaries. An unmatched quote can borrow a quote from the next declaration, an unclosed shape can absorb every later edge, and `%% TODO: support [later` suppresses the rest of the graph because comments are filtered only after delimiter tracking. | Comments reset before state tracking; unresolved EOF state recovers into physical statements. Exact later nodes and edges are pinned. |
| Medium | Pipe-label closing is quadratic: every candidate pipe allocates and rescans its remaining suffix. Release measurements for 32/64/128/256 KB adversarial labels were 0.253/0.969/3.849/15.143 seconds. The heuristic also mistakes an arrow-like substring after an inner pipe for the next edge. | A single-pass scanner preserves embedded, chained, and arrow-like labels. The 64 KiB debug regression fell from 21.6 seconds to well under its 500 ms budget. |

## Review coverage

- Exact issue #40 flowchart cases in parser, Unicode, and ASCII output.
- Statement isolation across comments, malformed shapes, unmatched quotes, and
  later valid declarations.
- Balanced quoted shaped labels containing structural delimiters and fan-out
  characters.
- Pipe labels with embedded pipes, chained labeled edges, malformed input, and
  adversarial size scaling.
- Confirmed block-diagram first-pipe variant, version/lockfile consistency,
  changelogs, snapshots, dependency policy, secrets, and workflow impact.

## Regression evidence at reviewed head

- `a["left [ bracket"] --> b[End]` rendered one absorbed node and zero edges.
- `%% TODO: support [later` followed by `A --> B` rendered an empty graph.
- `a[Alpha "Beta]`, then `b["Gamma"]`, removed node `b` by borrowing its quote.
- A 256 KB pipe-rich label took about 15 seconds in release mode, scaling near
  4× for each 2× input increase.
