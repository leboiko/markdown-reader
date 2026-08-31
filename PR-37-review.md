# PR #37 review — YAML/TOML frontmatter

## Executive summary

PR #37 correctly centralizes pulldown-cmark options and covers the reported
YAML sequence/key rendering failure across the TUI, HTML export, link checker,
and section extractor. The rebased implementation compiles on current
`master`, preserves metadata as visible highlighted content, escapes exported
HTML through the existing syntect/HTML paths, and keeps mid-document rules and
unterminated blocks as ordinary Markdown.

One merge-blocking correctness finding remains in the line-based `--section`
frontmatter detector. It duplicates only part of pulldown-cmark's metadata
grammar, so the same document is interpreted differently across surfaces.

## Findings

### Major — `--section` disagrees with pulldown-cmark metadata boundaries

- Location: `src/section_extract.rs:48`
- Category: correctness / parser consistency
- Impact: YAML metadata closed with the standard `...` delimiter is scanned as
  Markdown, allowing a `#` comment inside metadata to shadow the real section.
  Conversely, a `---` opener followed by a blank first body line is rejected by
  pulldown-cmark but accepted by the handwritten detector, hiding a genuine
  heading from `--section`.
- Evidence: the strong regression tests
  `yaml_ellipsis_closed_frontmatter_is_skipped` and
  `blank_first_line_does_not_open_frontmatter` both fail on the reviewed code
  with exact first-heading mismatches. A no-op, dropping metadata, or blindly
  skipping more lines cannot satisfy both assertions.
- Recommended fix: derive the leading metadata span from pulldown-cmark's own
  `MetadataBlock` events/options rather than maintaining a second delimiter
  grammar, then retain both tests.

## Testing assessment

The existing 18 frontmatter tests are unusually strong: they pin the reported
key-after-sequence position, require content preservation, distinguish
frontmatter from Markdown chrome, validate source-line mapping, cover the TUI,
HTML export, link checking, section extraction, TOML, mid-document rules,
unterminated blocks, and slice reparsing. The two new negative boundary tests
close the remaining parser-equivalence gap.

## Architecture and design

Centralizing `markdown_options()` is the right boundary and avoids viewer,
exporter, and link-scanner drift. Reusing the renderer's existing verbatim-code
path keeps syntax highlighting and source mapping localized. The section
extractor is the sole exception because it reimplemented metadata recognition;
the finding above should remove that duplicate grammar.

## Red-team checklist

- Data safety: checked. The change is read-only document rendering/scanning;
  there is no persistence, schema, migration, or cache compatibility surface.
- Concurrency: checked. No threads, tasks, caches, or retry paths are added.
- Runtime degradation: checked. Parsing remains local and bounded by document
  size; malformed and unterminated metadata falls back to Markdown.
- Security/config: checked. Metadata is untrusted file content. TUI rendering
  emits ratatui spans; HTML rendering uses syntect's escaping or the existing
  `html_escape` fallback. No shell, network, path, secret, or unsafe boundary is
  introduced.
- Performance: checked. Each existing surface already parses the document once;
  the option flags do not add unbounded work. Section extraction remains linear.
- Tests: checked. Negative, malformed, mid-document, position, preservation,
  mapping, and cross-surface cases were inspected. The two missing grammar
  boundary cases are reproduced as failing tests.

## Evidence inspected

- Full rebased diff and surrounding implementations in all eight changed files.
- pulldown-cmark 0.13.4 metadata scanner semantics, including YAML `...`
  closure and the non-empty first body-line requirement.
- `cargo check --workspace --offline`: pass after the rebase/version update.
- `cargo test section_extract::tests -- --nocapture`: 14 pass, 2 intentionally
  failing boundary reproductions before the fix.
- `git diff --check`: pass.

## Recommendation

The finding was fixed after review by deriving the metadata span from
pulldown-cmark's own `MetadataBlock` event range. Both failing regressions and
all focused frontmatter suites now pass. Merge after the full CI-equivalent
suite and fresh GitHub checks pass on the rebased branch.

## Resolution

| Finding | Status | Verification |
|---|---|---|
| `--section` metadata grammar drift | Fixed | The `...`-closure and blank-first-line tests changed from exact-output failures to passes; focused renderer, HTML, link, and section suites also pass. |
