#!/usr/bin/env bash
# Install once: rustup toolchain install nightly --component rust-src
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target-asan}"
export RUSTFLAGS='-Zsanitizer=address -Cforce-frame-pointers=yes'
export RUSTDOCFLAGS='-Zsanitizer=address'
export ASAN_OPTIONS='detect_leaks=1:halt_on_error=1'
target=$(rustc +nightly -vV | sed -n 's/^host: //p')
# macOS DNS worker shutdown produces an LSan report even in a std-only
# detached-thread reproducer (see docs/release-1.36.0.md). Keep address
# checking across the entire suite; enable leak checking for isolated soaks.
ASAN_OPTIONS=detect_leaks=0:halt_on_error=1 cargo +nightly test -Zbuild-std --target "$target" --workspace
cargo +nightly test -Zbuild-std --target "$target" --bin markdown-reader memory_soak -- --ignored --test-threads=1
