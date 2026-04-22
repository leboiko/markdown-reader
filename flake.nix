{
  description = "markdown-reader — TUI markdown viewer with mermaid rendering, tabs, vim-mode editor, and live reload";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

        markdown-reader = pkgs.rustPlatform.buildRustPackage {
          pname = "markdown-reader";
          # Bin name; the workspace package is `markdown-tui-explorer`.
          version = cargoToml.package.version;

          src = pkgs.lib.cleanSource ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # Build only the parent crate; the workspace also contains
          # `mermaid-text` which would otherwise build its own bin.
          cargoBuildFlags = [ "--package" "markdown-tui-explorer" ];

          # Tests run in the Nix sandbox. The `open_link_picker_real_doc_repro`
          # test reads an absolute path on the maintainer's machine and is
          # `#[ignore]` by default, so it's already skipped. Everything else
          # is pure data + tempfiles and works in the sandbox.
          doCheck = true;
          cargoTestFlags = [ "--workspace" ];

          meta = with pkgs.lib; {
            description = cargoToml.package.description;
            homepage = "https://github.com/leboiko/markdown-reader";
            license = licenses.mit;
            maintainers = [ ];
            mainProgram = "markdown-reader";
            platforms = platforms.unix;
          };
        };
      in {
        packages.default = markdown-reader;
        packages.markdown-reader = markdown-reader;

        apps.default = {
          type = "app";
          program = "${markdown-reader}/bin/markdown-reader";
        };

        # `nix develop` opens a shell with everything needed for
        # contributing: rustc/cargo + the side tools our CI runs.
        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.rustc
            pkgs.cargo
            pkgs.rustfmt
            pkgs.clippy
            pkgs.cargo-deny
            pkgs.cargo-audit
          ];
        };
      });
}
