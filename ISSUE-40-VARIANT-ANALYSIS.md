# Issue #40 variant analysis

## Summary

| Field | Value |
|---|---|
| Original bug | Flowchart label delimiters were parsed as syntax (#40) |
| Analysis date | 2026-08-31 |
| Codebase | `leboiko/markdown-reader` |
| Confirmed variants | 1 |

## Original defect

Root cause: externally supplied Mermaid label text reached delimiter-based
statement and edge-label splitting without shape or operand awareness.

The original sites were `split_statements`' predecessor,
`try_consume_pipe_label`, and `extract_arrow_label` in
`crates/mermaid-text/src/parser/flowchart.rs`. Blind newline/semicolon splits
invented nodes from label fragments; first-pipe selection truncated labels;
and quote state could hide every later arrow.

## Search methodology

| Version | Pattern | Tool | Matches | True | False |
|---|---|---:|---:|---:|---:|
| v1 | exact `strip_prefix('|')` / `find('|')` pair | `rg` | 2 | 2 | 0 |
| v2 | all Rust `find`, `split`, and `rfind` uses of `|` | `rg` | 6 | 2 | 4 |
| v3 | blind newline-to-semicolon and semicolon splits | `rg` | 1 | 1 | 0 |

The broader pipe matches in arrow classification only locate a label suffix
already isolated by the tokenizer and are not statement boundaries. Quote
scanners in the common and Sankey parsers implement different grammars and do
not share this failure mode.

## Confirmed variant

### Block-diagram edge labels

| Severity | Confidence | Status |
|---|---|---|
| Low | High | Fixed |

`crates/mermaid-text/src/parser/block_diagram.rs::try_parse_edge` selected the
first pipe after `-->`. The public `render` entry point made it reachable from
arbitrary Mermaid input, so `A -->|one|two| B` became label `one` and target
`two| B`. Block diagrams permit one edge per line, making the final pipe the
unambiguous closing delimiter. An exact model-level regression now pins source,
target, and the complete `one|two` label.

## Prevention

The CI-ready guard is the regression suite rather than a textual lint: first-
delimiter selection is valid in unrelated grammars, while these tests exercise
the semantic invariant directly:

```sh
cargo test -p mermaid-text --test flowchart_parser_compat
cargo test -p mermaid-text parser::block_diagram::tests::parses_pipe_inside_edge_label
```
