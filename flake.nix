{
  # Cadenza build/test pipeline — Nix flake (N0 devShell + N1 runtime-store derivation).
  #
  # Increments of the Nix-flake pipeline migration (design: fleet/NIX-FLAKE-PIPELINE-SCOPING.md,
  # operator-committed 2026-08-02). The existing `xtask`/GHA pipeline stays the source of truth; the
  # flake makes each stage reproducible + shareable via the nix cache. Each derivation is TIGHTLY
  # SCOPED to exactly its package's inputs (operator directive 2026-08-02: fine-grained cache
  # invalidation), never a monolithic everything-depends-on-everything graph.
  #   N0  — `devShell` reproducing the CI toolchain (rustc pin + wasm-tools + cargo-component).
  #   N1  — `packages.runtime` + `packages.runtime-debug` + `packages.nfc` : the value-heap RELEASE and
  #         DEBUG-COUNTERS runtimes + the NFC component (cadenza:nfc, imported by the runtime, FINDING#23),
  #         built + stripped as NORMAL (input-addressed) derivations; `packages.*-hash` is the content
  #         address DERIVED from the built bytes (never asserted). `checks.*-hash-parity` verifies each
  #         equals its committed hash (REQUIRED_RUNTIME_HASH / DEBUG_RUNTIME_HASH / REQUIRED_NFC_HASH).
  #   N2  — `packages.reducer-guest` + `packages.cedar-guest` (+ `-hash` each) : the cdz-kernel
  #         reducer-guest and cdz-agent-host cedar-policy-guest wasm components built from source, same
  #         hash-falls-out shape.
  #   R2  — `packages.store` : every built component assembled into one content-addressed store dir
  #         (`<derived-hash>.wasm`), mirroring target/cadenza-store but built + addressed by nix.
  #   S1  — `packages.seed-compiler` : the NATIVE bootstrap toolchain (cdz + cdz-run binaries) via
  #         `buildRustPackage` (root Cargo.lock, tracked #1748). S2 cadenza-projects, S3 per-test skip.
  #   rcdzc-wasm — `packages.rcdzc-wasm` (+ `-hash`) : the compiler as a wasm32-wasip1 module for the
  #         agent kernel's blob store (v-agent-harness owns the store pointer + compile-effect ABI).
  #   S2  — `packages.example-project` (+ the `buildCadenzaProject` fn) : build a Cadenza project
  #         (Project.cdz + sources) through nix via the S1 compiler → its wasm.
  #   S3  — `testCadenzaProject` (+ `checks.example-project-tests`) : run a project's @tests through nix
  #         as a derivation — nix input-hashing SKIPS unchanged tests (cache hit), re-runs on a change.
  #         North star: nix builds every component + the compiler (native + wasm) + projects + runs tests
  #         with fine-grained skip-unchanged, all deterministically.
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

        # ── S1: the SEED COMPILER (native cdz/cdz-run toolchain) AS a derivation ──────────────────
        #
        # Operator arc (2026-08-03): after nixifying the wasm components, nix builds the native bootstrap
        # toolchain itself — the `cdz` unified CLI (compile/run/test/doctor) + the standalone `cdz-run`
        # runner. These are ordinary NATIVE Rust binaries (host target, NO build-std, NO wasm, no build.rs
        # / include_bytes!), members of the ROOT workspace (Cargo.toml `members =
        # ["implementation/seed/crates/*", "xtask"]`, resolver 3). The root Cargo.lock is now TRACKED
        # (#1748 — committed for determinism), so `rustPlatform.buildRustPackage` can vendor from it. We
        # bind the platform to the pinned `rustToolchain` (same rustc as CI, not nixpkgs default) and
        # restrict the build to the two seed-compiler binaries with `cargoBuildFlags`.
        seedRustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };
        # Scope src to the workspace BUILD INPUTS only (not the whole repo) so unrelated edits — spec
        # docs, guide, design, issues, fleet — don't bust this derivation's cache (fine-grained
        # invalidation, PR #1756 review). A workspace `buildRustPackage` needs every member crate present
        # (Cargo.toml `members = ["implementation/seed/crates/*", "xtask"]` must resolve even for `-p
        # cdz`), plus the root Cargo.toml/lock, the `.cargo` config, and the toolchain pin.
        seedSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
          ];
        };
        seedCompiler = seedRustPlatform.buildRustPackage {
          pname = "cdz-seed-compiler";
          version = "0.0.0";
          src = seedSrc;
          cargoLock.lockFile = ./Cargo.lock;
          # Build only the seed-compiler binaries, not the whole workspace (xtask etc.).
          cargoBuildFlags = [ "-p" "cdz" "-p" "cdz-run" ];
          # Tests run in the existing gate/CI, not here — this derivation just BUILDS the toolchain
          # reproducibly (S1). (S3 will add fine-grained per-test derivations.)
          doCheck = false;
        };

        # ── S2: build a CADENZA PROJECT through nix ───────────────────────────────────────────────
        #
        # Operator arc (2026-08-03): "then we can have it building cadenza projects." A reusable function
        # that runs the nix-built S1 `cdz` on a Cadenza project (`Project.cdz` + sources) → its compiled
        # wasm, as a derivation. `cdz build` compiles a project from source with JUST the compiler — no
        # value-heap store + no network at BUILD time (the emitted program imports the runtime by hash;
        # only `cdz run` needs the store). So the derivation is toolchain-only + fully hermetic.
        #   `src` : the project directory (must contain Project.cdz + the sources it names).
        #   output: $out/ holding the built artifacts (main.wasm + link-map.txt) `cdz build` emits.
        buildCadenzaProject = { pname, src }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";
            nativeBuildInputs = [ seedCompiler ];
            buildPhase = ''
              runHook preBuild
              set -o pipefail
              export HOME="$TMPDIR/home"; mkdir -p "$HOME"
              # Build THIS project explicitly (`.` = the unpacked src cwd) — never rely on `cdz`'s
              # upward `Project.cdz` search, which in a sandbox could escape to an unexpected parent
              # manifest (github-liaison #1779). The artifacts land beside the manifest in the cwd.
              cdz build .
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              cp ./*.wasm ./link-map.txt "$out"/ 2>/dev/null || cp ./*.wasm "$out"/
              runHook postInstall
            '';
          };

        # S2 gate witness: a minimal in-flake demo project (no committed .cdz needed) proving the S1
        # compiler builds a project through nix. Mirrors `cdz new`'s scaffold.
        exampleProjectSrc = pkgs.runCommand "cdz-example-project-src" { } ''
          mkdir -p "$out"
          cat > "$out/Project.cdz" <<'EOF'
          def name = "example"
          def entry = "main.cdz"
          def tests = ["main.cdz"]
          EOF
          cat > "$out/main.cdz" <<'EOF'
          def main() -> Int64 = 0

          @test
          def main_is_zero() = if main() == 0 then unit else trap("main")

          export { main }
          EOF
        '';
        exampleProject = buildCadenzaProject {
          pname = "cdz-example-project";
          src = exampleProjectSrc;
        };

        # ── S3: run a project's tests through nix, cached per-input (skip unchanged) ───────────────
        #
        # Operator arc (2026-08-03): "nix to run tests with relative fine granularity so we can skip
        # tests that haven't changed." `testCadenzaProject` runs the nix-built S1 `cdz test` on a project
        # as a DERIVATION — so nix's input-hashing gives the skip FOR FREE at derivation granularity: if
        # the project's test sources + the compiler + the store are unchanged, the derivation is a CACHE
        # HIT and the tests DON'T re-run; only a changed input re-runs them. `cdz test` compiles a test
        # component from the `@test`-marked defs and runs each; it needs the value-heap store at RUNTIME
        # (a heap-using test resolves the runtime by hash), so we point CDZ_STORE at the nix component
        # store. The derivation SUCCEEDS iff all tests pass (a failing `cdz test` exits non-zero → build
        # fails); the output is a small pass-marker. Fine-grained: one derivation per project (a per-file
        # split is a later refinement once projects have multiple independently-cacheable test files).
        testCadenzaProject = { pname, src }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";
            nativeBuildInputs = [ seedCompiler ];
            buildPhase = ''
              runHook preBuild
              # `set -o pipefail` is LOAD-BEARING here: without it, `cdz test | tee` adopts the LAST
              # command's status (tee — normally 0 for a healthy write), which MASKS an upstream
              # `cdz test` failure, so a FAILING suite would still yield a SUCCESSFUL derivation —
              # silently defeating the whole point of gating tests through nix (github-liaison #1786).
              # With pipefail, a non-zero `cdz test` propagates through the pipe and fails the build.
              set -o pipefail
              export HOME="$TMPDIR/home"; mkdir -p "$HOME"
              export CDZ_STORE="${componentStore}"
              # Test THIS project explicitly (`.` = the unpacked src cwd), not via the upward
              # manifest search — same sandbox-escape guard as buildCadenzaProject.
              cdz test . | tee "$TMPDIR/test.out"
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              # `cdz test` exits non-zero on failure (so a red suite already fails the build); record the
              # summary as the cached output — a cache HIT here means "these tests passed + haven't changed".
              cp "$TMPDIR/test.out" "$out"
              runHook postInstall
            '';
          };
        exampleProjectTests = testCadenzaProject {
          pname = "cdz-example-project-tests";
          src = exampleProjectSrc;
        };

        # ── rcdzc→WASM: the Cadenza COMPILER as a wasm artifact (agent-harness v0.2, operator 2026-08-03) ─
        #
        # v-agent-harness needs rcdzc as a content-addressable wasm the kernel loads from its blob store +
        # invokes on .cdz source → program wasm (operator: "compile rcdzc→wasm, agents compile+call cadenza
        # programs"). SPLIT (agreed): I own the BUILD (this derivation); they own the kernel store pointer +
        # the compile-effect ABI. The crate (`implementation/seed/crates/rcdzc-wasm`) is a cdylib built for
        # wasm32-wasip1 (a plain wasm MODULE, NOT a component — no wit/lift; the raw (ptr,len) export ABI is
        # theirs). It path-deps a 7-crate closure (rcdzc + cadenza-syntax + cadenza-ast + cdz-run + cdz-rt +
        # cdz-num), all of which must be in the source. NORMAL cargo build (no build-std) → vendor JUST its
        # own committed leaf lock via importCargoLock; wasip1 std comes from the toolchain.
        #   🪤 dontFixup = true: the output is a single wasm FILE; stdenv's fixupPhase runs `strip` on it
        #      (mis-detecting it as an ELF) and TRUNCATES it to ~54 bytes. Disabling fixup keeps the full
        #      ~5.3 MB artifact. (Verified: with fixup, out=54B; with dontFixup, out=5309060B.)
        rcdzcWasmVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/rcdzc-wasm/Cargo.lock;
        };
        rcdzcWasmSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (map (c: ./implementation/seed/crates + ("/" + c)) [
            "rcdzc-wasm" "rcdzc" "cadenza-syntax" "cadenza-ast" "cdz-run" "cdz-rt" "cdz-num"
          ] ++ [ ./rust-toolchain.toml ]);
        };
        rcdzcWasm = pkgs.stdenvNoCC.mkDerivation {
          pname = "rcdzc-wasm";
          version = "0.0.0";
          src = rcdzcWasmSrc;
          nativeBuildInputs = [ rustToolchain ];
          dontFixup = true; # a single wasm file — fixup's `strip` would truncate it (see note above).
          buildPhase = ''
            runHook preBuild
            export HOME="$TMPDIR/home"
            export CARGO_HOME="$TMPDIR/cargo"
            export CARGO_NET_OFFLINE=true
            mkdir -p "$HOME" "$CARGO_HOME"
            cat > "$CARGO_HOME/config.toml" <<EOF
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${rcdzcWasmVendor}"
            EOF
            cd implementation/seed/crates/rcdzc-wasm
            cargo build --release --target wasm32-wasip1 --locked
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp target/wasm32-wasip1/release/rcdzc_wasm.wasm "$out"
            runHook postInstall
          '';
        };

        # ── N1: the value-heap runtime components AS input-addressed derivations (hash from output) ─
        #
        # `xtask build` produces TWO runtime components (build_component + canonicalize_runtime in
        # xtask/src/main.rs): the RELEASE runtime (what a shipped program pins + composes) and the
        # DEBUG-COUNTERS runtime (same code + the `live-objects` leak counter, `--features
        # debug-counters`, that a Perceus leak-check harness composes). Each is `cargo component
        # build --release --target wasm32-unknown-unknown` on cdz-runtime (build-std +
        # panic=immediate-abort from its .cargo/config.toml, enabled on the stable pin via
        # RUSTC_BOOTSTRAP=1), then `wasm-tools strip -a` to drop the non-deterministic tool-version
        # `producers` sections. The stripped bytes ARE the content-addressed store artifact.
        #
        # DESIGN (operator north star 2026-08-03): the content hash is DERIVED FROM THE BUILT BYTES,
        # never asserted. So each runtime is a NORMAL (input-addressed) derivation — NOT a fixed-output
        # derivation with a pinned/read `outputHash`. The build produces the stripped wasm; the hash
        # FALLS OUT as `sha256(that output)`, exposed via `packages.*-hash`. Nothing in this file names
        # a 64-hex literal or reads one from `runtime_abi.rs` — `runtime_abi.rs`'s recorded hash becomes
        # a CONSUMER of this nix-built truth (a later increment inverts `xtask codegen` to read it),
        # not the source. The gate proving parity (nix hash == the committed REQUIRED_RUNTIME_HASH)
        # lives in `checks.*` below, comparing the DERIVED hash to what the build already records —
        # never a hand-maintained pin.
        #
        # A normal derivation has NO network, so cargo's deps are VENDORED offline (`importCargoLock`):
        #   - the runtime's own committed Cargo.lock (its crates.io closure), AND
        #   - the toolchain's rust-src `library/Cargo.lock` (build-std compiles core/alloc/panic_abort
        #     from source, and cargo resolves THEIR deps — e.g. libc — from that second lockfile; a
        #     hermetic sandbox has no `~/.cargo` cache, so this MUST be vendored too).
        # Both vendor dirs are merged (`symlinkJoin`) and wired via a `[source.crates-io]
        # replace-with` CARGO_HOME config; the build runs with `CARGO_NET_OFFLINE=true` (NOT the
        # `--offline` flag — see the mkStripComponent buildPhase note on why the flag breaks the NFC WIT
        # dep resolution).
        #
        # TIGHTLY SCOPED source: only the cdz-runtime crate (+ the workspace pin) — NOT the whole repo
        # — so a change ANYWHERE ELSE does not invalidate these derivations' cache.
        runtimeSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-runtime
            # The runtime's world imports `cadenza:nfc/normalize` (FINDING#23), and its Cargo.toml points
            # cargo-component's WIT resolution at the sibling NFC crate's WIT
            # (`[package.metadata.component.target.dependencies] "cadenza:nfc" = { path = "../cdz-nfc/wit" }`).
            # So the NFC WIT dir MUST be in the build source or `cargo component build` can't resolve the
            # dep offline. Scope to just the WIT (not the whole cdz-nfc crate) — that's all the runtime
            # build reads.
            ./implementation/seed/crates/cdz-nfc/wit
            ./rust-toolchain.toml
          ];
        };

        # Merged offline vendor dir: the runtime's crates.io deps + the toolchain's build-std deps.
        runtimeVendor = pkgs.symlinkJoin {
          name = "cdz-runtime-cargo-vendor";
          paths = [
            (pkgs.rustPlatform.importCargoLock {
              lockFile = ./implementation/seed/crates/cdz-runtime/Cargo.lock;
            })
            (pkgs.rustPlatform.importCargoLock {
              # build-std's own lockfile, shipped inside the pinned toolchain derivation.
              lockFile = "${rustToolchain}/lib/rustlib/src/rust/library/Cargo.lock";
            })
          ];
        };

        # Build a build-std wasm component (runtime OR the NFC component) as a NORMAL (input-addressed)
        # derivation — the hash is derived from its output, never asserted. Parametrized over:
        #   crateDir : path under implementation/seed/crates (e.g. "cdz-runtime", "cdz-nfc")
        #   artifact : the produced .wasm stem (e.g. "cdz_runtime", "cdz_nfc")
        #   src      : the tightly-scoped fileset source for this crate
        #   vendor   : its merged offline cargo vendor dir (own lock + rust-src build-std lock)
        #   features : cargo `--features` list (release = [], debug = ["debug-counters"])
        mkStripComponent = { pname, crateDir, artifact, src, vendor, features ? [ ] }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";

            nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ];

            featuresArg = pkgs.lib.optionalString (features != [ ])
              ("--features " + pkgs.lib.concatStringsSep "," features);

            buildPhase = ''
              runHook preBuild
              export RUSTC_BOOTSTRAP=1
              export HOME="$TMPDIR/home"
              export CARGO_HOME="$TMPDIR/cargo"
              mkdir -p "$HOME" "$CARGO_HOME"
              # Point cargo at the merged offline vendor dir (crates.io + build-std deps).
              cat > "$CARGO_HOME/config.toml" <<EOF
              [source.crates-io]
              replace-with = "vendored-sources"
              [source.vendored-sources]
              directory = "${vendor}"
              EOF
              cd implementation/seed/crates/${crateDir}
              # --locked honors the committed Cargo.lock exactly. Network is blocked by CARGO_NET_OFFLINE
              # (set below) + the sandbox itself — NOT the `--offline` FLAG: the runtime's world imports
              # the NFC component (a `[package.metadata.component.target.dependencies]` WIT path-dep on
              # ../cdz-nfc/wit, FINDING#23), and the `--offline` flag makes `cargo component` refuse that
              # component-dep resolution outright ("lock file must be provided when offline mode is
              # enabled") even though it's a LOCAL path needing no network. CARGO_NET_OFFLINE blocks the
              # crates.io registry (our vendor covers it) while still letting cargo-component resolve the
              # local WIT dep. A truly-missing dep still fails LOUD (no network in the sandbox).
              export CARGO_NET_OFFLINE=true
              cargo component build --release --target wasm32-unknown-unknown --locked $featuresArg
              runHook postBuild
            '';

            # CANONICALIZE (strip the tool-version producers sections) — the same step xtask's
            # canonicalize_runtime does. The stripped bytes are the content-addressed artifact.
            installPhase = ''
              runHook preInstall
              wasm-tools strip -a \
                target/wasm32-unknown-unknown/release/${artifact}.wasm \
                -o "$out"
              runHook postInstall
            '';
          };

        # The value-heap runtime derivations bind mkStripComponent to the cdz-runtime crate.
        mkRuntime = { pname, features }:
          mkStripComponent {
            inherit pname features;
            crateDir = "cdz-runtime";
            artifact = "cdz_runtime";
            src = runtimeSrc;
            vendor = runtimeVendor;
          };

        # The RELEASE runtime — what a shipped program pins (REQUIRED_RUNTIME_HASH).
        runtime = mkRuntime {
          pname = "cdz-runtime-component";
          features = [ ];
        };

        # The DEBUG-COUNTERS runtime — same code + the `live-objects` leak counter
        # (`--features debug-counters`); the Perceus leak-check harness composes it (DEBUG_RUNTIME_HASH).
        runtimeDebug = mkRuntime {
          pname = "cdz-runtime-component-debug";
          features = [ "debug-counters" ];
        };

        # ── N1: the NFC component (`cdz-nfc`) AS an input-addressed derivation (hash from output) ──
        #
        # FINDING#23: the runtime's world imports `cadenza:nfc/normalize` by hash, so the heavy Unicode
        # normalization tables live in a SEPARATE component the runtime composes from the CAS. `xtask
        # build` stores it beside the runtimes; codegen records its hash as `REQUIRED_NFC_HASH`. It's
        # RUNTIME-SHAPED (build-std + panic=immediate-abort + canonicalize/strip), so it reuses
        # mkStripComponent. Its own WIT (`wit/nfc.wit`, `package cadenza:nfc`) is self-contained (no
        # `[metadata.component.target.dependencies]`), so its fileset is just the cdz-nfc crate. build-std
        # → the same 2-lockfile vendor as the runtime (its own Cargo.lock + rust-src's).
        nfcVendor = pkgs.symlinkJoin {
          name = "cdz-nfc-cargo-vendor";
          paths = [
            (pkgs.rustPlatform.importCargoLock {
              lockFile = ./implementation/seed/crates/cdz-nfc/Cargo.lock;
            })
            (pkgs.rustPlatform.importCargoLock {
              lockFile = "${rustToolchain}/lib/rustlib/src/rust/library/Cargo.lock";
            })
          ];
        };
        nfcSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-nfc
            ./rust-toolchain.toml
          ];
        };
        nfc = mkStripComponent {
          pname = "cdz-nfc-component";
          crateDir = "cdz-nfc";
          artifact = "cdz_nfc";
          src = nfcSrc;
          vendor = nfcVendor;
        };

        # ── N2: the reducer-guest wasm COMPONENT as a content-addressed derivation ────────────────
        #
        # `cdz-kernel`'s component_reducer_e2e / async_component_reducer_e2e tests load a wasm-component
        # reducer fixture. Today that's a COMMITTED binary (reducer_guest.component.wasm, `include_bytes!`);
        # this builds it from source instead (operator "stop committing wasm"), so the committed binary
        # can be deleted and the test reads the nix-built path (a companion cdz-kernel change wires the
        # env var). Same normal-derivation, hash-falls-out shape as the runtimes — but SIMPLER:
        #   - the guest is a plain `cargo build` (NOT `cargo component`, NO build-std), so vendoring is
        #     just its OWN committed Cargo.lock (one importCargoLock, no rust-src std lock).
        #   - `wit_bindgen::generate!` reads `../../../wit/reducer.wit` at compile time, so the source
        #     fileset MUST include `cdz-kernel/wit` alongside the guest crate.
        #   - the artifact is produced by `wasm-tools component new` (the LIFT of the core module into a
        #     component) — NOT `strip`; the lifted component IS the content-addressed output.
        reducerGuestVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-guest/Cargo.lock;
        };
        reducerGuestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-guest
            ./implementation/seed/crates/cdz-kernel/wit
            ./rust-toolchain.toml
          ];
        };
        reducerGuest = pkgs.stdenvNoCC.mkDerivation {
          pname = "reducer-guest-component";
          version = "0.0.0";
          src = reducerGuestSrc;
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools ];
          buildPhase = ''
            runHook preBuild
            export HOME="$TMPDIR/home"
            export CARGO_HOME="$TMPDIR/cargo"
            mkdir -p "$HOME" "$CARGO_HOME"
            cat > "$CARGO_HOME/config.toml" <<EOF
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${reducerGuestVendor}"
            EOF
            cd implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-guest
            cargo build --release --target wasm32-unknown-unknown --locked --offline
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            wasm-tools component new \
              target/wasm32-unknown-unknown/release/reducer_guest.wasm \
              -o "$out"
            runHook postInstall
          '';
        };

        # ── N2: the cedar-policy-guest wasm COMPONENT as a content-addressed derivation ───────────
        #
        # `cdz-agent-host`'s cedar_authz_e2e test drives a Cedar authorizer built from a wit-bindgen
        # guest that embeds the real Cedar decision engine (`cedar-policy = "4"`). That component is
        # ~3.3 MB, so it was NEVER committed — CI builds it and hands the path to the test via
        # CEDAR_POLICY_COMPONENT (an optional-skip env; the test skips locally when unset). This builds
        # it as a derivation instead, so the nix store serves it (no per-consumer rebuild). Same shape
        # as reducer-guest, with two differences:
        #   - the WIT is INSIDE the guest crate (`wit_bindgen::generate!({ path: "wit/authorizer.wit" })`),
        #     so the fileset is JUST the guest dir — no separate wit dir.
        #   - a 173-package Cedar-engine closure (still one importCargoLock, plain cargo build, no
        #     build-std). ~37s cold; cached after.
        cedarGuestVendor = pkgs.rustPlatform.importCargoLock {
          lockFile =
            ./implementation/seed/crates/cdz-agent-host/tests/fixtures/cedar-policy-guest/Cargo.lock;
        };
        cedarGuestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-agent-host/tests/fixtures/cedar-policy-guest
            ./rust-toolchain.toml
          ];
        };
        cedarGuest = pkgs.stdenvNoCC.mkDerivation {
          pname = "cedar-policy-guest-component";
          version = "0.0.0";
          src = cedarGuestSrc;
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools ];
          buildPhase = ''
            runHook preBuild
            export HOME="$TMPDIR/home"
            export CARGO_HOME="$TMPDIR/cargo"
            mkdir -p "$HOME" "$CARGO_HOME"
            cat > "$CARGO_HOME/config.toml" <<EOF
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${cedarGuestVendor}"
            EOF
            cd implementation/seed/crates/cdz-agent-host/tests/fixtures/cedar-policy-guest
            cargo build --release --target wasm32-unknown-unknown --locked --offline
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            wasm-tools component new \
              target/wasm32-unknown-unknown/release/cedar_policy_guest.wasm \
              -o "$out"
            runHook postInstall
          '';
        };

        # The content address of a built component = sha256 of its (stripped) bytes. DERIVED from the
        # artifact nix built — this is the Cadenza content-address a program pins, falling out of the
        # build rather than being asserted. Exposed as a `packages.*-hash` (a plain-text store file).
        hashOf = drv: name:
          pkgs.runCommand name { } ''
            ${pkgs.coreutils}/bin/sha256sum ${drv} | ${pkgs.coreutils}/bin/cut -d' ' -f1 \
              | ${pkgs.coreutils}/bin/tr -d '\n' > $out
          '';

        # ── R2: the content-addressed component STORE ─────────────────────────────────────────────
        #
        # Assemble every nix-built component into ONE store directory, each file named by its DERIVED
        # content hash: `<sha256>.wasm`. This mirrors `target/cadenza-store` (what `xtask build`
        # produces) but built + addressed BY NIX — the store the operator's north star describes, from
        # which a cadenza runtime / the harness loads a component by hash. Purely a function of the
        # component derivations, so it's cache-shareable + rebuilt only when a component changes.
        # (A later increment has the runtime/harness RESOLVE from this store; that's a cross-territory
        # change coordinated with v-runtime + the harness — this increment only PRODUCES the store.)
        componentStore = pkgs.runCommand "cdz-component-store" { } ''
          set -euo pipefail
          mkdir -p "$out"
          for c in ${runtime} ${runtimeDebug} ${nfc} ${reducerGuest} ${cedarGuest}; do
            h=$(${pkgs.coreutils}/bin/sha256sum "$c" | ${pkgs.coreutils}/bin/cut -d' ' -f1)
            ${pkgs.coreutils}/bin/cp "$c" "$out/$h.wasm"
          done
        '';
      in
      {
        # N1: the value-heap runtime components as NORMAL (input-addressed) derivations — `nix build
        # .#runtime` / `.#runtime-debug` builds + strips the wasm; `.#runtime-hash` / `.#runtime-debug-hash`
        # is the DERIVED content address (sha256 of the built bytes), the value a program pins. The hash
        # is never asserted here — it falls out of the build (operator north star). Parity with the
        # committed REQUIRED_RUNTIME_HASH is a `checks` assertion below, not a pin.
        packages.runtime = runtime;
        packages.runtime-debug = runtimeDebug;
        packages.runtime-hash = hashOf runtime "cdz-runtime-hash";
        packages.runtime-debug-hash = hashOf runtimeDebug "cdz-runtime-debug-hash";

        # N1: the NFC component (`cdz-nfc`) the runtime imports by hash (REQUIRED_NFC_HASH). `.#nfc` is
        # the stripped component; `.#nfc-hash` its derived content address.
        packages.nfc = nfc;
        packages.nfc-hash = hashOf nfc "cdz-nfc-hash";

        # N2: the reducer-guest wasm component, built from source (replaces the committed binary).
        # `.#reducer-guest` is the lifted component; `.#reducer-guest-hash` its derived content address.
        packages.reducer-guest = reducerGuest;
        packages.reducer-guest-hash = hashOf reducerGuest "reducer-guest-hash";

        # N2: the cedar-policy-guest wasm component (never committed — CI-built ~3.3 MB). `.#cedar-guest`
        # is the lifted authorizer component; `.#cedar-guest-hash` its derived content address. A later
        # increment points cdz-agent-host's CEDAR_POLICY_COMPONENT at this store path.
        packages.cedar-guest = cedarGuest;
        packages.cedar-guest-hash = hashOf cedarGuest "cedar-guest-hash";

        # R2: the content-addressed component store — every nix-built component as `<derived-hash>.wasm`
        # in one dir (mirrors target/cadenza-store, but built + addressed by nix). `nix build .#store`.
        packages.store = componentStore;

        # S1: the native seed compiler (cdz + cdz-run). `nix build .#seed-compiler` → result/bin/{cdz,cdz-run}.
        packages.seed-compiler = seedCompiler;

        # rcdzc→wasm: the compiler as a wasm artifact for the agent kernel's blob store. `.#rcdzc-wasm`
        # is the wasm module; `.#rcdzc-wasm-hash` its derived content address (for v-agent-harness's
        # compiler-latest store pointer).
        packages.rcdzc-wasm = rcdzcWasm;
        packages.rcdzc-wasm-hash = hashOf rcdzcWasm "rcdzc-wasm-hash";

        # S2: build a Cadenza project through nix (the S1 compiler on Project.cdz → wasm).
        # `.#example-project` is the gate-witness demo, built by the in-flake `buildCadenzaProject`
        # function (reusable — point it at any project dir; a cross-system `lib` export can wrap it later).
        packages.example-project = exampleProject;

        # S3: run a project's tests through nix, cached per-input (skip unchanged). `.#example-project-tests`
        # is the witness (built by `testCadenzaProject`). Also a `checks` entry so `nix flake check` runs it.
        packages.example-project-tests = exampleProjectTests;

        # PARITY CHECK (not a pin): assert the DERIVED hash of the nix-built runtime equals the hash
        # `xtask codegen` already recorded in runtime_abi.rs. This reads the committed value only to
        # COMPARE — the flake never uses it as the build's asserted output. It catches a divergence
        # between the nix build and the xtask build (e.g. a toolchain/vendor drift) at `nix flake
        # check` time. `runtime_abi.rs` is `@generated by cargo xtask codegen`; we extract the 64-hex
        # literal following the named `pub const … : &str =` declaration (guarded: the split MUST match,
        # else we THROW rather than compare against a stray literal; case-insensitive hex).
        checks =
          let
            abi = builtins.readFile
              ./implementation/seed/crates/rcdzc/src/backend/wasm/runtime_abi.rs;
            recordedHash = constName:
              let
                decl = "pub const " + constName + ": &str =";
                parts = builtins.split decl abi;
                afterDecl = if builtins.length parts >= 3 then pkgs.lib.last parts else null;
                m = if afterDecl == null then null
                    else builtins.match "[^\"]*\"([0-9a-fA-F]{64})\".*" afterDecl;
              in
              if afterDecl == null then
                throw "flake.nix: `${decl}` not found in runtime_abi.rs (codegen shape changed?)"
              else if m == null then
                throw "flake.nix: `${decl}` found but no 64-hex literal followed it"
              else builtins.head m;
            parity = { name, drv, constName }:
              pkgs.runCommand "${name}-hash-parity" { } ''
                got=$(${pkgs.coreutils}/bin/sha256sum ${drv} | ${pkgs.coreutils}/bin/cut -d' ' -f1)
                want=${recordedHash constName}
                if [ "$got" != "$want" ]; then
                  echo "PARITY FAIL: nix-built ${name} hash $got != runtime_abi.rs ${constName} $want" >&2
                  exit 1
                fi
                echo "ok: nix-built ${name} == ${constName} ($want)" > $out
              '';
            # VALIDITY: assert a built artifact is a well-formed wasm COMPONENT. The guest derivations
            # end in `wasm-tools component new` (the lift); nothing else gates that the result is valid,
            # so a future guest/WIT/toolchain change could silently produce a broken component that only
            # blows up at test-load time. This check fails the flake at `nix flake check` instead.
            validComponent = { name, drv }:
              pkgs.runCommand "${name}-valid" { nativeBuildInputs = [ pkgs.wasm-tools ]; } ''
                wasm-tools validate --features component-model ${drv}
                echo "ok: nix-built ${name} is a valid wasm component" > $out
              '';
          in
          {
            runtime-hash-parity = parity {
              name = "runtime"; drv = runtime; constName = "REQUIRED_RUNTIME_HASH";
            };
            runtime-debug-hash-parity = parity {
              name = "runtime-debug"; drv = runtimeDebug; constName = "DEBUG_RUNTIME_HASH";
            };
            nfc-hash-parity = parity {
              name = "nfc"; drv = nfc; constName = "REQUIRED_NFC_HASH";
            };
            reducer-guest-valid = validComponent { name = "reducer-guest"; drv = reducerGuest; };
            cedar-guest-valid = validComponent { name = "cedar-guest"; drv = cedarGuest; };
            # S3: the example project's @tests run through nix — a cache HIT when its sources are
            # unchanged (the "skip tests that haven't changed" win), a re-run + fail on a red test.
            example-project-tests = exampleProjectTests;
          };

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

          # R4: point cdz/cdz-run at the NIX-BUILT component store. cdz-run + cdz `default_store()`
          # resolve `CDZ_STORE` (env) before the compiled `target/cadenza-store` fallback (the --store
          # flag still wins over the env); the content-address re-hash-verify on load is untouched, so a
          # wrong store entry is caught, not silently loaded. So exporting CDZ_STORE=<packages.store>
          # makes `cdz run`/`cdz test` inside `nix develop` resolve every component (runtime + NFC +
          # guests) from the nix-built, content-addressed store — the operator's load-by-hash north star.
          # OPT-IN + non-destructive: `cargo xtask build` (the store WRITER) still writes
          # target/cadenza-store; this only overrides the READ path for a nix-develop session.
          shellHook = ''
            export CDZ_STORE="${componentStore}"
            echo "cdz: CDZ_STORE → nix component store ($CDZ_STORE)"
          '';
        };
      });
}
