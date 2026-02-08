{
  description = "Cadenza";

  nixConfig = {
    extra-substituters = [ "https://camshaft.cachix.org" ];
    extra-trusted-public-keys = [ "camshaft.cachix.org-1:NuMo5iCUNwDpNWJNlhCw/nFp3aQ7sxsVBXdlNtXs3CQ=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    kframework.url = "github:runtimeverification/k";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, kframework, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system: {
      devShells.default = let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default kframework.overlays.default ];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in pkgs.mkShell {
        buildInputs = with pkgs; [
          # Rust
          rust
          cargo-watch
          cargo-expand
          
          # TypeScript
          deno

          # Python
          uv

          # Documentation
          mdbook
          
          # K Framework
          k

          # Development tools
          git
          nixpkgs-fmt
        ];

        shellHook = ''
          echo "Cadenza development environment"
          echo "Rust: $(rustc --version)"
          echo "Deno: $(deno --version)"
          echo "UV: $(uv --version)"
          echo "K Framework: $(kompile --version 2>&1 | head -n1 || echo 'not available')"
        '';
      };
    });
}
