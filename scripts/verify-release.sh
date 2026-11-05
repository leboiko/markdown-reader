#!/usr/bin/env bash
# Local release gates; run from any directory. Each gate must succeed.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo doc --no-deps --workspace
cargo deny check
