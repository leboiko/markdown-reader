# Nix flake — release notes

The Nix flake at the repository root (`flake.nix`) tracks the **current
master branch**. There is no separate release step for Nix users — when
they run `nix profile upgrade`, they pull whatever is on master.

This is unlike Homebrew (which pins a specific GitHub Release) and AUR
(which pins a specific tag): Nix users get rolling updates by default,
and version-pin via their own `flake.lock`.

## What ships

- `nix run github:leboiko/markdown-reader` — one-off run.
- `nix profile install github:leboiko/markdown-reader` — install into
  the user's profile.
- Embedded as an input in another flake — `inputs.markdown-reader.url
  = "github:leboiko/markdown-reader"`.
- `nix develop github:leboiko/markdown-reader` — dev shell with the
  Rust toolchain + linters/auditors our CI uses.

## How it builds

`flake.nix` invokes `pkgs.rustPlatform.buildRustPackage` with:

- `pname = "markdown-reader"` (binary name, even though the workspace
  package is `markdown-tui-explorer`)
- `version` parsed from `Cargo.toml`'s `[package]` block
- `cargoLock.lockFile = ./Cargo.lock` so Nix prefetches every crate
  before the sandboxed build (no network access during `cargo build`)
- `cargoBuildFlags = [ "--package" "markdown-tui-explorer" ]` so the
  workspace-sibling `mermaid-text` crate doesn't get its own bin in
  the output
- `cargoTestFlags = [ "--workspace" ]` runs every test in the sandbox.
  The single `#[ignore]`-d test that reads an absolute path on a
  maintainer machine is already skipped by the harness.

## CI

`.github/workflows/nix.yml` runs on every push/PR that touches a
flake-relevant file (flake itself, Cargo files, source). The workflow:

1. Installs Nix via `DeterminateSystems/nix-installer-action`.
2. Caches the Nix store via `DeterminateSystems/magic-nix-cache-action`
   (faster repeat builds across PRs).
3. Runs `nix flake check --no-build` (cheap structural validation).
4. Runs `nix build .#markdown-reader` on `ubuntu-latest` and
   `macos-latest`.
5. Smoke-tests the resulting binary with `--help`.

A failed Nix build is **blocking on PR** — caught before merge.

## Updating dependencies

For end users, `nix flake update` in their own flake re-pins them to
our latest master. We don't need to do anything on our side beyond
landing changes on master.

For our `flake.lock`, updates happen via:

```sh
nix flake update                      # bump all inputs
nix flake update --update-input nixpkgs   # bump just one
```

Commit `flake.lock` after updates so CI tests against the same Nix
revision contributors do.

## Submitting to nixpkgs (optional, future)

Once the project is more stable and has a steady user base, we may
submit a derivation to `nixpkgs` itself so users can `nix-env -iA
markdown-reader` without enabling flakes. That's a separate workflow
involving a PR to `NixOS/nixpkgs` — not part of our regular release
process.

## Troubleshooting

- **"experimental Nix feature 'flakes' is disabled"** — user needs
  `experimental-features = nix-command flakes` in `~/.config/nix/nix.conf`.
- **Build fails with a network error** — `cargoLock` is the culprit;
  ensure `Cargo.lock` is committed and up to date.
- **Build succeeds but binary segfaults at startup** — likely a glibc
  vs musl mismatch on the user's host. Run `ldd` against the binary
  to inspect linker requirements.
