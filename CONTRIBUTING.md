# Contributing

Thanks for contributing to `markdown-reader`.

## Development setup

```sh
git clone https://github.com/leboiko/markdown-reader.git
cd markdown-reader
cargo build
```

Optional Nix shell:

```sh
nix develop
```

## Before opening a PR

Run the usual checks:

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

If your change touches packaging, releases, or dependency policy, also run:

```sh
cargo deny check
cargo audit
```

## What makes a good PR

- Keep the scope tight
- Add or update tests when behavior changes
- Update the README when user-facing behavior changes
- Prefer repo patterns over introducing a new style or abstraction

## Areas where help is especially useful

- Markdown rendering correctness
- Table layout and wrapping
- Mermaid text/image fallback behavior
- Search, navigation, and source-line mapping
- Packaging and terminal compatibility

## Reporting bugs

Use the issue templates when possible. For security-sensitive issues, use the
process in [SECURITY.md](SECURITY.md).
