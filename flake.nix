{
  # Cadenza build/test pipeline — Nix flake (N0 devShell + N1 runtime-store derivation).
  #
  # Increments of the Nix-flake pipeline migration (design: fleet/NIX-FLAKE-PIPELINE-SCOPING.md,
  # operator-committed 2026-08-02). The existing `xtask`/GHA pipeline stays the source of truth; the
  # flake makes each stage reproducible + shareable via the nix cache. Each derivation is TIGHTLY
  # SCOPED to exactly its package's inputs (operator directive 2026-08-02: fine-grained cache
  # invalidation), never a monolithic everything-depends-on-everything graph.
  #   N0  — `devShell` reproducing the CI toolchain (rustc pin + wasm-tools + cargo-component).
  #   N1  — `packages.runtime` + `packages.runtime-debug` : the value-heap RELEASE and
  #         DEBUG-COUNTERS runtime components, built + stripped + content-addressed AS fixed-output
  #         derivations (this file, below). N2 guest-wasm, N3 tests follow.
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

        # ── N1: the value-heap runtime components AS content-addressed derivations ────────────────
        #
        # `xtask build` produces TWO runtime components (build_component + canonicalize_runtime in
        # xtask/src/main.rs): the RELEASE runtime (what a shipped program pins + composes) and the
        # DEBUG-COUNTERS runtime (same code + the `live-objects` leak counter, `--features
        # debug-counters`, that a Perceus leak-check harness composes). Each is `cargo component
        # build --release --target wasm32-unknown-unknown` on cdz-runtime (build-std +
        # panic=immediate-abort from its .cargo/config.toml, enabled on the stable pin via
        # RUSTC_BOOTSTRAP=1), then `wasm-tools strip -a` to drop the non-deterministic tool-version
        # `producers` sections. The stripped bytes ARE the store artifact + the thing SHA-256'd into
        # REQUIRED_RUNTIME_HASH / DEBUG_RUNTIME_HASH respectively.
        #
        # Each is a FIXED-OUTPUT DERIVATION whose `outputHash` IS the recorded content hash: the
        # Cadenza content-address is sha256(stripped bytes), and a flat FOD's OUTPUT CONTENT hash is
        # sha256(output file bytes), so when the output IS exactly the stripped runtime they coincide.
        # This makes Nix ITSELF enforce the parity gate the design doc calls most fragile — if the
        # build ever produces different bytes the derivation FAILS to realize (content-hash mismatch),
        # so a drift can never silently ship. FOD also grants the network `cargo` needs to fetch the
        # (Cargo.lock-pinned) deps, without vendoring them in this slice.
        #
        # TIGHTLY SCOPED inputs: only the cdz-runtime crate source (+ the workspace pin file) — NOT
        # the whole repo — so a change ANYWHERE ELSE does not invalidate these derivations' cache.
        runtimeSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-runtime
            ./rust-toolchain.toml
          ];
        };

        # Read the expected content hash from the SINGLE SOURCE OF TRUTH rather than hard-coding a
        # literal here (operator directive 2026-08-03: don't hard-code hashes in the flake — a pinned
        # literal is a second place to hand-maintain, and drifts/churns on every runtime-bytes change).
        # `runtime_abi.rs` is `@generated by cargo xtask codegen` and holds `pub const
        # REQUIRED_RUNTIME_HASH`/`DEBUG_RUNTIME_HASH: &str = "<64-hex>";`; we regex-extract the literal
        # that immediately follows the named `pub const` declaration. So when codegen re-records a hash
        # (a runtime change), the FOD's outputHash tracks it automatically — no edit to this file.
        runtimeAbi = builtins.readFile
          ./implementation/seed/crates/rcdzc/src/backend/wasm/runtime_abi.rs;
        hashFromAbi = constName:
          let
            # split on the exact `pub const NAME: &str =` declaration; the text AFTER it is the last
            # split element (the file has the const once). Then take the first 64-hex string literal.
            afterDecl = pkgs.lib.last (builtins.split ("pub const " + constName + ": &str =") runtimeAbi);
            m = builtins.match "[^\"]*\"([0-9a-f]{64})\".*" afterDecl;
          in
          if m == null then
            throw "flake.nix: could not read ${constName} from runtime_abi.rs (codegen shape changed?)"
          else
            builtins.head m;

        # Build the value-heap runtime component as a fixed-output derivation. `features` is the cargo
        # `--features` list (release = [], debug = ["debug-counters"]); `hashConst` names the
        # `runtime_abi.rs` const whose recorded content address the stripped bytes MUST reproduce
        # (read at eval time, NOT hard-coded) — it becomes the FOD's outputHash.
        mkRuntime = { pname, features, hashConst }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname;
            version = "0.0.0";
            src = runtimeSrc;

            nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ];

            # Flat FOD: the output IS the content-addressed stripped runtime, so its CONTENT hash
            # (sha256 of the file bytes) is the recorded hash — read from runtime_abi.rs, not pinned.
            # Nix verifies it — this IS the parity gate. (The nix store-path hash is a distinct
            # derivation-derived hash — not this.)
            outputHashMode = "flat";
            outputHashAlgo = "sha256";
            outputHash = hashFromAbi hashConst;

            featuresArg = pkgs.lib.optionalString (features != [ ])
              ("--features " + pkgs.lib.concatStringsSep "," features);

            buildPhase = ''
              runHook preBuild
              export RUSTC_BOOTSTRAP=1
              # HOME must be writable for cargo/cargo-component caches inside the sandbox.
              export HOME="$TMPDIR/home"
              mkdir -p "$HOME"
              cd implementation/seed/crates/cdz-runtime
              # --locked: honor the COMMITTED Cargo.lock exactly (the runtime's lock IS committed and
              # pins deps). Without it `cargo` may re-resolve to different dep versions and rewrite the
              # lock, which would (a) undermine this FOD's determinism and (b) waste a network fetch
              # before the output-hash check fails. --locked fails LOUDLY on a stale lock instead.
              cargo component build --release --target wasm32-unknown-unknown --locked $featuresArg
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

        # The RELEASE runtime — what a shipped program pins (REQUIRED_RUNTIME_HASH).
        runtime = mkRuntime {
          pname = "cdz-runtime-component";
          features = [ ];
          hashConst = "REQUIRED_RUNTIME_HASH";
        };

        # The DEBUG-COUNTERS runtime — same code + the `live-objects` leak counter
        # (`--features debug-counters`); the Perceus leak-check harness composes it (DEBUG_RUNTIME_HASH).
        runtimeDebug = mkRuntime {
          pname = "cdz-runtime-component-debug";
          features = [ "debug-counters" ];
          hashConst = "DEBUG_RUNTIME_HASH";
        };
      in
      {
        # N1: the value-heap runtime components, content-addressed. `nix build .#runtime` /
        # `.#runtime-debug` realizes the stripped runtime; because each is a fixed-output derivation,
        # Nix checks the output's CONTENT hash (sha256 of the file bytes) == the recorded hash
        # (REQUIRED_RUNTIME_HASH / DEBUG_RUNTIME_HASH) and fails the build on any drift. (The nix
        # STORE-PATH hash is a different, derivation-derived hash — do NOT locate the artifact by the
        # content hash; the content hash is what's enforced.)
        packages.runtime = runtime;
        packages.runtime-debug = runtimeDebug;

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
