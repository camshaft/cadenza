{
  description = "Cadenza";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system: {
      devShells.default = let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
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
          
          # System dependencies
          libfabric
          rdma-core
          
          # Development tools
          git
          nixpkgs-fmt
        ];

        shellHook = ''
          echo "Cadenza development environment"
          echo "Rust: $(rustc --version)"
          echo "Deno: $(deno --version)"
          echo "UV: $(uv --version)"
        '';
      };
    });
}
