{
  # Cadenza build/test pipeline — Nix flake (N0 devShell + N1 runtime-store derivation).
  #
  # Increments of the Nix-flake pipeline migration (design: fleet/NIX-FLAKE-PIPELINE-SCOPING.md,
  # operator-committed 2026-08-02). The existing `xtask`/GHA pipeline stays the source of truth; the
  # flake makes each stage reproducible + shareable via the nix cache. Each derivation is TIGHTLY
  # SCOPED to exactly its package's inputs (operator directive 2026-08-02: fine-grained cache
  # invalidation), never a monolithic everything-depends-on-everything graph.
  #   N0  — `devShell` reproducing the CI toolchain (rustc pin + wasm-tools + cargo-component).
  #   N1  — `packages.runtime` : the value-heap RELEASE runtime component, built + stripped +
  #         content-addressed AS a derivation (this file, below). N2 guest-wasm, N3 tests follow.
  #
  # The Rust toolchain is read DIRECTLY from `rust-toolchain.toml` (the load-bearing pin — the
  # recorded `REQUIRED_RUNTIME_HASH` is only reproducible on that exact rustc). `rust-toolchain.toml`
  # stays the single source of truth; the flake CONSUMES it via oxalica's rust-overlay, so there is
  # no second place to bump the version.

  description = "Cadenza build/test pipeline (Nix flake: N0 devShell + N1 runtime derivation)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Reproduce the EXACT pinned toolchain from rust-toolchain.toml (channel 1.95.0 + rustc,
        # cargo, clippy, rustfmt, rust-src + the wasm32-unknown-unknown target). Reading the file
        # means the pin never drifts from what `cargo`/CI use.
        #
        # We ADD `wasm32-wasip1` on top of the file's `targets` via `.override`. rust-toolchain.toml
        # stays the single source of truth for the CHANNEL + components + the primary
        # wasm32-unknown-unknown target; wasm32-wasip1 is an extra target the devShell needs but the
        # file doesn't list, because it's only required by `cargo component`'s bindings-generation
        # step (it invokes the toolchain for wasm32-wasip1 before producing the final
        # wasm32-unknown-unknown component). A rustup host has wasm32-wasip1 installed globally so the
        # gap is invisible there; a hermetic Nix devShell must add it explicitly or `cargo component
        # build` fails with "failed to find the `wasm32-wasip1` target". Adding a target does NOT
        # change the emitted wasm32-unknown-unknown bytes (and thus does not affect
        # REQUIRED_RUNTIME_HASH) — it only makes the extra target's std available to the bindings step.
        rustToolchain =
          (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
            targets = [ "wasm32-unknown-unknown" "wasm32-wasip1" ];
          };

        # ── N1: the value-heap RELEASE runtime component AS a derivation ──────────────────────────
        #
        # This wraps exactly what `xtask build` does for the release runtime (build_component +
        # canonicalize_runtime in xtask/src/main.rs): `cargo component build --release --target
        # wasm32-unknown-unknown` on cdz-runtime (with build-std + panic=immediate-abort from its
        # .cargo/config.toml, enabled on the stable pin via RUSTC_BOOTSTRAP=1), then `wasm-tools
        # strip -a` to drop the non-deterministic tool-version `producers` sections. The stripped
        # bytes ARE the store artifact + the thing SHA-256'd into REQUIRED_RUNTIME_HASH.
        #
        # Modeled as a FIXED-OUTPUT DERIVATION whose `outputHash` IS REQUIRED_RUNTIME_HASH: the
        # Cadenza content-address is sha256(stripped bytes), and a flat FOD's hash is sha256(output
        # file bytes), so when the output IS exactly the stripped runtime the two hashes coincide.
        # This makes Nix ITSELF enforce the parity gate the design doc calls most fragile — if the
        # build ever produces different bytes, the derivation FAILS to realize (hash mismatch), so a
        # drift can never silently ship. FOD also grants the network access `cargo` needs to fetch
        # the (Cargo.lock-pinned) deps, without vendoring them in this slice.
        #
        # TIGHTLY SCOPED inputs: only the cdz-runtime crate source (+ the workspace pin files) — NOT
        # the whole repo — so a change ANYWHERE ELSE does not invalidate this derivation's cache.
        runtimeSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-runtime
            ./rust-toolchain.toml
          ];
        };

        requiredRuntimeHash =
          "90f09723549a9658fa209ed6c3483032199ee31d965b515b21c51f6b7f7ebc7a";

        runtime = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-runtime-component";
          version = "0.0.0";
          src = runtimeSrc;

          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ];

          # A fixed-output derivation: the output IS the content-addressed stripped runtime, so its
          # hash is the recorded REQUIRED_RUNTIME_HASH (flat sha256 of the file bytes). Nix verifies
          # the realized output matches — this IS the parity gate.
          outputHashMode = "flat";
          outputHashAlgo = "sha256";
          outputHash = requiredRuntimeHash;

          buildPhase = ''
            runHook preBuild
            export RUSTC_BOOTSTRAP=1
            # HOME must be writable for cargo/cargo-component caches inside the sandbox.
            export HOME="$TMPDIR/home"
            mkdir -p "$HOME"
            cd implementation/seed/crates/cdz-runtime
            cargo component build --release --target wasm32-unknown-unknown
            runHook postBuild
          '';

          # CANONICALIZE (strip the tool-version producers sections) — the same step xtask's
          # canonicalize_runtime does before hashing. The stripped bytes are the flat FOD output.
          installPhase = ''
            runHook preInstall
            wasm-tools strip -a \
              target/wasm32-unknown-unknown/release/cdz_runtime.wasm \
              -o "$out"
            runHook postInstall
          '';
        };
      in
      {
        # N1: the value-heap release runtime component, content-addressed. `nix build .#runtime`
        # realizes it into a store path whose hash is REQUIRED_RUNTIME_HASH — a drift fails to build.
        packages.runtime = runtime;

        devShells.default = pkgs.mkShell {
          # TIGHTLY SCOPED: only what the seed workspace's build/gate actually needs —
          #   rustToolchain : rustc/cargo/clippy/rustfmt/rust-src + wasm32 target (from the pin)
          #   wasm-tools    : CI installs this per wasm job (checks.yml `wasm-tools: true`); the
          #                   runtime component build + `cdz test` need it. Pin it from nixpkgs.
          #   cargo-component : the runtime component build is `cargo component build` (see xtask
          #                   build_component_with_features). Without it in the shell, `cargo
          #                   component` leaks from the host `~/.cargo/bin`, defeating hermeticity.
          #                   nixpkgs pins 0.21.1 — the exact version the recorded REQUIRED_RUNTIME_HASH
          #                   was produced with, so the devShell build reproduces the committed hash.
          # NOT added: anything a specific later derivation needs — those go in that derivation's
          # own inputs (N1+), not this shared shell, to keep cache invalidation fine-grained.
          packages = [
            rustToolchain
            pkgs.wasm-tools
            pkgs.cargo-component
          ];
        };
      });
}
