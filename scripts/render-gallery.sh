#!/usr/bin/env bash
# Render every mermaid block in docs/mermaid-gallery.md through the
# release `mermaid-text` binary and emit the results to a single file.
#
# Use this after any change that may affect rendering (renderer, layout,
# glyph alphabet, parser, anything that touches `crates/mermaid-text/`)
# to visually scan for regressions that unit/snapshot tests miss.
#
# Why this exists: snapshot tests can silently pin a regression — the
# canonical mindmap snapshot pinned the buggy "trunk dropped, children
# at column 0" output for 13 patch versions. A 30-second visual sweep
# of the gallery output catches the class of bug that survives every
# unit and snapshot gate.
#
# Usage:
#     scripts/render-gallery.sh                # writes /tmp/gallery_render.txt
#     scripts/render-gallery.sh path/to/out    # writes to a custom path
#
# The script always builds the release binary first to ensure the output
# reflects the current source. It exits non-zero on build failure but is
# tolerant of per-diagram render failures (those are reported inline so
# you can spot which diagram broke).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GALLERY="$REPO_ROOT/docs/mermaid-gallery.md"
BIN="$REPO_ROOT/target/release/mermaid-text"
OUT="${1:-/tmp/gallery_render.txt}"

if [ ! -f "$GALLERY" ]; then
    echo "error: gallery not found at $GALLERY" >&2
    exit 1
fi

echo "Building release binary..." >&2
cargo build --release -p mermaid-text --bin mermaid-text --quiet

: > "$OUT"

# Extract every ```mermaid block plus the most recent ## heading above
# it, then render each block through the release binary. Heading + first
# line of source give enough context to pair the rendered output with
# the gallery section it came from.
awk '
    /^## / { current_heading = $0 }
    /^```mermaid$/ { in_block = 1; block = ""; next }
    /^```$/ && in_block {
        printf "===HEADING===\n%s\n===BLOCK===\n%s===END===\n", current_heading, block
        in_block = 0
        next
    }
    in_block { block = block $0 "\n" }
' "$GALLERY" | awk -v BIN="$BIN" -v OUT="$OUT" '
    /^===HEADING===$/ { mode = "heading"; next }
    /^===BLOCK===$/ { mode = "block"; block = ""; next }
    /^===END===$/ {
        n++
        printf "\n\n========================================================================\n" >> OUT
        printf "Diagram %d under heading: %s\n", n, heading >> OUT
        printf "------------------------------------------------------------------------\n" >> OUT
        first_line = block
        sub(/\n.*/, "", first_line)
        printf "First line of source: %s\n", first_line >> OUT
        printf "------------------------------------------------------------------------\n" >> OUT
        # Stage the source in a tmp file so awk-getline complications are
        # avoided. Stderr is captured so render errors land in the report.
        tmp_in = "/tmp/gallery_block.mmd"
        printf "%s", block > tmp_in
        close(tmp_in)
        rendered_cmd = BIN " < " tmp_in " 2>&1"
        while ((rendered_cmd | getline line) > 0) {
            printf "%s\n", line >> OUT
        }
        close(rendered_cmd)
        next
    }
    mode == "heading" { heading = $0 }
    mode == "block" { block = block $0 "\n" }
'

echo "Rendered $(grep -c '^Diagram ' "$OUT") diagram(s) to $OUT" >&2
echo "Open the file and visually scan each render — pay attention to:" >&2
echo "  - mindmaps: trunk must connect to first child" >&2
echo "  - flowcharts: no orphan blank rows above content" >&2
echo "  - state diagrams: composite borders close cleanly" >&2
echo "  - sequence diagrams: lifelines align under participant boxes" >&2
echo "  - any diagram type you specifically modified the renderer for" >&2
