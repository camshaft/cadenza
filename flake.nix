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
  #   R2  — `packages.store` : every built component assembled into one content-addressed store dir
  #         (`<derived-hash>.wasm`), mirroring target/cadenza-store but built + addressed by nix.
  #   S1  — `packages.seed-compiler` : the NATIVE bootstrap toolchain (cdz + cdz-run binaries) via
  #         `buildRustPackage` (root Cargo.lock, tracked #1748). S2 cadenza-projects, S3 per-test skip.
  #   rcdzc-wasm — `packages.rcdzc-wasm` (+ `-hash`) : the compiler as a wasm32-wasip1 module.
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
    # crane: a cargo-artifact CACHE — `buildDepsOnly` compiles the workspace's DEPENDENCY closure once into a
    # content-addressed derivation, so each per-crate clippy/test check recompiles only FIRST-PARTY sources
    # instead of the whole dep tree every run (operator fleet-velocity mandate: push CI as low as possible).
    # crane is nixpkgs-lib-only → no `inputs.nixpkgs.follows`.
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane }:
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

        # crane bound to OUR pinned toolchain (NOT crane's default) so its clippy/test match rust-toolchain.toml
        # exactly — same rustc as the cargo path + the codegen job that records REQUIRED_RUNTIME_HASH.
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # ── build FRAMEWORK helper (operator seq-126: DRY the per-build boilerplate) ───────────────
        #
        # `mkCargoVendorEnv { vendor }` emits the shell preamble every cargo-in-nix derivation shares:
        # set HOME/CARGO_HOME, write the offline `config.toml` (jobs cap + source-replacement), block the
        # network. Replaces the ~dozen hand-rolled copies. Two vendor shapes:
        #   - SINGLE importCargoLock vendor (merged = false, the default): the vendor dir SHIPS its own
        #     `.cargo/config.toml` carrying ALL source replacements — crates-io ALWAYS, plus a
        #     `[source."git+…#sha"] … replace-with` stanza WHEN the lock has a git dep. So we SOURCE that
        #     config (rewrite its relative `directory = "cargo-vendor-dir"` → the absolute store path,
        #     prepend our [build] jobs cap) rather than hand-roll a crates-io-only one. This makes a git
        #     dependency Just Work: the derivation inherits the vendor's git source-replacement stanza,
        #     so cargo resolves the git crate from the vendor instead of the network (the offline-mode
        #     fetch failure that bit cdz-agent-host's s2n-quic-dc-metrics dep). Verified byte-identical to
        #     the old hand-rolled crates-io-only config for a no-git vendor (the config only affects
        #     source RESOLUTION, not the built bytes).
        #   - symlinkJoin-MERGED vendor (merged = true): several importCargoLock outputs joined into one
        #     dir; their per-vendor `.cargo/config.toml`s collide in the join (one wins), so we can't
        #     source a single authoritative one — hand-roll the crates-io config pointing `directory` at
        #     the join. All merged vendors today are crates-io-only (build-std + component-dep locks); a
        #     merged vendor that ever needs a git stanza would need it merged in explicitly (flag if so).
        mkCargoVendorEnv = { vendor, merged ? false }:
          if merged then ''
            export HOME="$TMPDIR/home"
            export CARGO_HOME="$TMPDIR/cargo"
            export CARGO_NET_OFFLINE=true
            mkdir -p "$HOME" "$CARGO_HOME"
            cat > "$CARGO_HOME/config.toml" <<EOF
            [build]
            jobs = 4
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${vendor}"
            EOF
          '' else ''
            export HOME="$TMPDIR/home"
            export CARGO_HOME="$TMPDIR/cargo"
            export CARGO_NET_OFFLINE=true
            mkdir -p "$HOME" "$CARGO_HOME"
            {
              echo "[build]"
              echo "jobs = 4"
              sed 's|directory = "cargo-vendor-dir"|directory = "${vendor}"|' \
                "${vendor}/.cargo/config.toml"
            } > "$CARGO_HOME/config.toml"
          '';

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
        # The cdz+cdz-run DEP-CLOSURE (seq-126 crateClosure walk) — the only crates whose src the seed
        # compiler actually compiles. includeOptional=false (v-nix+v-cml 2026-08-10): the seed cdz is built
        # `--no-default-features` (below), so the default-on `corpus` feature is OFF → cdz-corpus is NOT
        # linked → its src must ALSO leave this closure (else a corpus-only MR rotates seedCompilerSrc +
        # needlessly re-runs the cad-test-compiler-ml spine; pr-sync throughput flag). So now 9 of the 18
        # seed crates (cadenza-ast/syntax, cdz, cdz-calc, cdz-num, cdz-rt, cdz-run, cdz-rust-render, rcdzc);
        # cdz-corpus JOINS the OUTSIDE/stubbed set (+ cdz-agent-host, cdz-cad, cdz-kernel, cdz-nfc,
        # cdz-runtime, cdz-smith, cdz-wasm, rcdzc-wasm, xtask). The corpus subcommand is unavailable in the
        # test-runner cdz — fine, cdz test never invokes it (v-cml confirmed corpus-independent).
        seedCompilerClosure = pkgs.lib.unique (
          crateClosure' { includeOptional = false; } "cdz"
          ++ crateClosure' { includeOptional = false; } "cdz-run");
        # seedCompilerSrc: SCOPED to the cdz+cdz-run closure so an edit to a NON-closure seed crate (cdz-kernel,
        # cdz-agent-host, cdz-corpus's siblings, etc. — the fleet commits to these every few min) does NOT rotate
        # this derivation's input hash → the /nix/store cache actually WARMS across candidates. Was `seedSrc` (all
        # 18 crates → any seed-crate commit busted it → cache-miss every run → cdz-agent-host/cad-tests/genesis
        # never warmed, since they depend on cdz via `cdz compile` — v-nix+v-ft 2026-08-06). Same isolation the
        # per-crate crane checks use: FULL src/ for closure crates + Cargo.toml-only for non-closure members
        # (+ xtask) + synthetic stubs (postPatch) so cargo's workspace `members` glob resolves for `-p cdz`
        # without their real src. `.cargo`/lock/toolchain pinned. fmt keeps the broad seedSrc (it needs all src).
        seedCompilerSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (pkgs.lib.concatMap crateCompileSrc seedCompilerClosure)
            ++ nonClosureManifests seedCompilerClosure
            ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]);
        };
        # CRANE-INCREMENTAL (v-nix+v-ft 2026-08-08, cold-warm/saturation arc): built via craneLib consuming the
        # SHARED cargoArtifacts deps-layer instead of a monolithic buildRustPackage. WHY: the old buildRustPackage
        # recompiled the ENTIRE dep closure + first-party crates on EVERY rcdzc/cdz/cdz-run edit (rcdzc changes
        # ~every commit — heavy active dev), so the seed-compiler drv rebuilt cold 15-19m per candidate, and
        # cad-tests/cdz-agent-host (which bear it) held a runner slot that long, saturating the pool during bursts
        # (trigger (c) confirmed from 3 seats). Consuming cargoArtifacts (the warm-keep-pinned deps layer, rotates
        # only on Cargo.lock ~= never) means deps RESTORE warm + only the changed first-party crates recompile →
        # <2m incremental → each slot frees ~8x faster → more candidates/hr through the fixed pool. Same scoping as
        # craneCrateCommon (seedCompilerSrc fileset + stubNonClosure + seedCargoVendor), just multi-crate
        # (cdz+cdz-run) via cargoExtraArgs. Output contract UNCHANGED: $out/bin/{cdz,cdz-run}.
        seedCompiler = craneLib.buildPackage {
          pname = "cdz-seed-compiler";
          version = "0.0.0";
          src = seedCompilerSrc;
          inherit cargoArtifacts;
          cargoVendorDir = seedCargoVendor;
          # Materialize synthetic empty target stubs for the non-closure members (+ xtask) whose real src the
          # scoped fileset omits, so cargo can parse the workspace `members` glob for `-p cdz -p cdz-run`
          # without their src. Content-fixed → invariant to those crates' real edits (the whole point). chmod
          # first: fileset.toSource copies are read-only. Same stub machinery as the per-crate crane checks.
          # preBuild (crane's hook — runs after crane restores cargoArtifacts' target/, before the build).
          preBuild = ''
            # INJECT the nix-built runtime/nfc content hashes into the compiler's `option_env!` reads
            # (rcdzc `runtime_abi.rs`), so this `cdz` STAMPS the exact runtime/nfc it will run against in
            # THIS closure — not the platform-specific literal `codegen` last committed. The value-heap
            # runtime's wasm is not byte-reproducible across host platforms (build-std + wasm codegen), so
            # a committed hash drifts on any host but the one that recorded it; deriving the hash HERE, from
            # the component nix built beside this compiler, makes the pair self-consistent on EVERY host and
            # retires the cross-host reproducibility requirement. Read at build time via `cdz-contract blob`
            # (no IFD — the runtime crosses as a build-time file dependency, not an eval-time string), the
            # same content-address the store keys by and a guest's `cadenza:runtime/heap@…+<hash>` import
            # names, so the guest resolves its heap dependency from the seeded CAS. The hashes come from the
            # shared `hashOf` derivations (`runtimeHash`/`runtimeDebugHash`/`nfcHash`) — computed ONCE and
            # `cat` here — not a fresh `cdz-contract blob` per consumer.
            export CDZ_RUNTIME_HASH="$(cat ${runtimeHash})"
            export CDZ_DEBUG_RUNTIME_HASH="$(cat ${runtimeDebugHash})"
            export CDZ_NFC_HASH="$(cat ${nfcHash})"
            chmod -R u+w .
            ${stubNonClosure seedCompilerClosure}
            [ -f xtask/src/main.rs ] || { mkdir -p xtask/src; echo "fn main(){}" > xtask/src/main.rs; }
            [ -f xtask/src/lib.rs ] || echo "" > xtask/src/lib.rs
          '';
          # Build only the seed-compiler binaries, not the whole workspace (xtask etc.). crane injects --locked
          # + --release; cargoExtraArgs adds the -p scoping (crane's equivalent of buildRustPackage cargoBuildFlags).
          # --no-default-features drops cdz's default-on `corpus` feature (v-nix+v-cml 2026-08-10): the test-runner
          # cdz doesn't need the corpus subcommand (cdz test is corpus-independent, v-cml confirmed), and dropping
          # it removes cdz-corpus from this build's closure — paired with seedCompilerClosure's includeOptional=false
          # (which drops cdz-corpus SRC from the fileset), a corpus-only MR no longer rotates seedCompiler → the
          # cad-test-compiler-ml spine no longer over-triggers on corpus MRs (pr-sync throughput flag). cdz-run has
          # no default features to drop, so --no-default-features is a no-op for it. The hard-gate STILL fires on
          # rcdzc/Core/compiler-ml edits (those ARE in the closure) — only the corpus false-trigger is removed.
          cargoExtraArgs = "-p cdz -p cdz-run --no-default-features";
          # Build only — tests run in the existing gate/CI (S1: reproducible toolchain build). Do NOT re-export
          # the deps layer (we consume the shared cargoArtifacts, not produce a new one).
          doCheck = false;
          doInstallCargoArtifacts = false;
        };

        # ── Full-CI-in-nix (operator GO 2026-08-04): re-express each GHA `checks.yml` job as a nix
        # derivation so the WHOLE CI is runnable inside nix (replacing the one-off scripts + brittle
        # hand-wiring), then cut over. Incremental — one job-class per increment, each ADVISORY
        # (continue-on-error in checks.yml) until v-fleet-tooling flips the required-set cutover.
        #
        # `cargoWorkspaceCheck` runs ONE cargo command against the scoped seed workspace source, hermetic
        # + offline-vendored. `seedCargoVendor` vendors the root lock offline (a normal derivation has no
        # network); the pinned `rustToolchain` carries rustfmt + clippy (from the toolchain file's
        # components). Used for the pure-native-workspace checks — those needing NO runtime store + NO
        # wasm — so each reproduces EXACTLY the matching GHA job, now cached by nix:
        #   Increment 1 — `fmt` (cargo fmt --all --check) + `clippy` (cargo clippy --workspace …).
        #   Increment 2 — `test` (cargo test --workspace).
        seedCargoVendor = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };
        # `test` (`cargo test --workspace`) reads more of the repo at RUN time than fmt/clippy do, so its
        # src is WIDER than `seedSrc` (crates + xtask). fmt/clippy keep the narrow `seedSrc` for finer
        # cache invalidation (a spec/compiler-ml edit shouldn't bust lint). The extra paths:
        #   spec/semantics            — cadenza-syntax's corpus_roundtrip tests resolve
        #                               `$CARGO_MANIFEST_DIR/../../../../spec/semantics`. Scoped to
        #                               semantics/ (not all of spec/) so a spec/design|capabilities edit
        #                               doesn't bust the test-check cache — other spec refs in the seed
        #                               are compile-time duvet `//=` citations, not runtime reads
        #                               (github-liaison #1989).
        #   implementation/compiler-ml — cdz's run_ml_cli tests shell out to `cdz run-ml`, which locates
        #                               `implementation/compiler-ml/src` and writes a pid-stamped driver
        #                               INTO it. nix's unpackPhase copies src into the WRITABLE build dir,
        #                               so the driver write succeeds there (the driver is .gitignored +
        #                               cleaned up on exit).
        seedTestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./implementation/compiler-ml
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
            ./spec/semantics
          ];
        };
        # `roundtrip` (`cargo xtask roundtrip`) is corpus-only — it reads spec/semantics but NOT
        # compiler-ml (that's only for `cargo test`'s run_ml_cli). So it gets a NARROWER src than
        # seedTestSrc (drops ./implementation/compiler-ml) — a compiler-ml edit shouldn't bust the
        # roundtrip check's cache (github-liaison #2007).
        seedRoundtripSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
            ./spec/semantics
          ];
        };
        cargoWorkspaceCheck = { name, cargoCmd, src ? seedSrc, extraInputs ? [ ] }:
          pkgs.stdenvNoCC.mkDerivation {
            pname = name;
            version = "0.0.0";
            inherit src;
            nativeBuildInputs = [ rustToolchain ] ++ extraInputs;
            buildPhase = ''
              runHook preBuild
              # Network is blocked by CARGO_NET_OFFLINE (set by mkCargoVendorEnv, belt-and-suspenders with
              # the vendored source) so if cargo's source resolution ever changes the lint fails LOUDLY
              # offline instead of attempting a fetch (github-liaison #1982). seedCargoVendor is a single
              # importCargoLock (no git deps) → merged = false (source its config).
              ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
              ${cargoCmd}
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              echo "ok: ${name} (${cargoCmd})" > "$out"
              runHook postInstall
            '';
          };

        # ── seq-126 Part B: PER-CRATE dependency-granularity (concierge-confirmed, option A + 1a) ───
        #
        # REPLACE the whole-workspace `clippy --workspace` / `test --workspace` (which bust on ANY
        # root-crate edit — the coarse spot) with PER-CRATE checks (`clippy -p C` + `test -p C`). What a
        # SHARED cargo workspace permits (concierge-confirmed 2026-08-04):
        #   - INDEPENDENT crates don't cross-trigger: editing X's src re-runs only the per-crate checks
        #     whose CLOSURE contains X (editing cdz-num does NOT re-run cadenza-syntax's check).
        #   - TEST-dir churn is isolated: each check ships ONLY its own crate's tests/, so editing a
        #     crate's tests/ re-runs only its check (the high-frequency win).
        #   - A src edit correctly re-runs DEPENDENTS (a real dep change — not a shortfall).
        # We can't scope below "all member src + this crate's tests": `cargo -p C` LOADS the whole
        # workspace, and loading requires every member's Cargo.toml to PARSE (target auto-detection needs
        # each member's src/lib.rs present). `fmt --all` stays whole-workspace (inherently whole-tree +
        # cheap). Coverage parity: every workspace test binary maps to one member crate (verified via
        # `cargo test --workspace --no-run`) → ∪ `test -p C` == the old whole-workspace run.
        rootWorkspaceCrates = {
          cadenza-ast = "implementation/seed/crates/cadenza-ast";
          cadenza-syntax = "implementation/seed/crates/cadenza-syntax";
          cdz = "implementation/seed/crates/cdz";
          cdz-calc = "implementation/seed/crates/cdz-calc";
          # cdz-component-rewrite: the isolated wasm-component import re-addresser (bare import -> +hash),
          # used by `cargo xtask build`. A ROOT workspace member (no own [workspace]), so — like cdz-contract
          # below — it MUST be registered here or the crane deps-layer src omits its Cargo.toml and the whole
          # workspace fails to load.
          cdz-component-rewrite = "implementation/seed/crates/cdz-component-rewrite";
          # cdz-contract (#3026): the dep-minimal content-hash + contract-id crate cdz-platform now depends on.
          # A ROOT workspace member (no own [workspace]), so it MUST be registered here — else the crane
          # deps-layer src omits its Cargo.toml and the whole workspace fails to load (`cargo check` can't
          # read a member's manifest). The other crates/* dirs absent from this map are standalone [workspace]s.
          cdz-contract = "implementation/seed/crates/cdz-contract";
          cdz-platform = "implementation/seed/crates/cdz-platform";
          cdz-corpus = "implementation/seed/crates/cdz-corpus";
          cdz-corpus-grade = "implementation/seed/crates/cdz-corpus-grade";
          cdz-num = "implementation/seed/crates/cdz-num";
          cdz-rt = "implementation/seed/crates/cdz-rt";
          cdz-run = "implementation/seed/crates/cdz-run";
          cdz-rust-render = "implementation/seed/crates/cdz-rust-render";
          # cdz-world-artifact: the isolated WIT-world → KIND_WIT_WORLD binary-AST utility, shelled out to by
          # the `worldArtifacts` derivation (and `cargo xtask world-artifact`). A ROOT workspace member (no own
          # [workspace]), so — like cdz-component-rewrite / cdz-contract — it MUST be registered here or the
          # crane deps-layer src omits its Cargo.toml and the whole workspace fails to load.
          cdz-world-artifact = "implementation/seed/crates/cdz-world-artifact";
          cdz-rust-run = "implementation/seed/crates/cdz-rust-run";
          # cdz-wasm-opt-gap (#4537): the std-only, zero-dep parse+format bin for the wasm-opt optimality-gap
          # sweep (its per-case Nix derivation runs wasm-opt; this just formats the record). A ROOT workspace
          # member (no own [workspace]), so — like cdz-contract / cdz-world-artifact — it MUST be registered
          # here or the crane deps-layer src omits its Cargo.toml and the whole workspace fails to load.
          cdz-wasm-opt-gap = "implementation/seed/crates/cdz-wasm-opt-gap";
          rcdzc = "implementation/seed/crates/rcdzc";
          xtask = "xtask";
        };
        rootCrateNames = builtins.attrNames rootWorkspaceCrates;
        # direct member-edges of one crate across the three rebuild-relevant dep sections (A1 walk).
        # `includeOptional` (default true) keeps the historical behaviour — count every path-dep edge. Pass
        # false to EXCLUDE `optional = true` deps: used ONLY by seedCompilerClosure, which builds cdz with
        # `--no-default-features` (v-nix+v-cml 2026-08-10, corpus-over-trigger fix). WHY it matters: an
        # optional path-dep like cdz-corpus (cdz/Cargo.toml, behind the default-on `corpus` feature) STILL
        # has a `path` attr, so the unfiltered walk pulls its SOURCE into seedCompilerSrc's fileset → a
        # corpus-only MR rotates seedCompilerSrc → rebuilds seedCompiler → re-runs the ~28min cad-test-
        # compiler-ml spine, even though the corpus-off seedCompiler build doesn't link cdz-corpus at all
        # (pr-sync throughput flag). Dropping the optional edge for the seed closure removes cdz-corpus src
        # from the fileset AND stubs it → a corpus edit no longer rotates seedCompiler. The DEFAULT-feature
        # consumers (per-crate crane checks, crateCdzCheck) keep includeOptional=true so their fileset still
        # carries corpus src (they build WITH default features + genuinely need it). Only the seed closure,
        # paired with the --no-default-features build below, drops it.
        crateDirectDeps = { includeOptional ? true }: name:
          let
            manifest = builtins.fromTOML
              (builtins.readFile (./. + "/${rootWorkspaceCrates.${name}}/Cargo.toml"));
            depsIn = section: manifest.${section} or { };
            edgesIn = section:
              builtins.filter (d: builtins.elem d rootCrateNames)
                (builtins.filter
                  (d:
                    let v = (depsIn section).${d}; in
                    builtins.isAttrs v && (v ? path)
                    && (includeOptional || !(v.optional or false)))
                  (builtins.attrNames (depsIn section)));
          in
          pkgs.lib.unique (pkgs.lib.concatMap edgesIn
            [ "dependencies" "dev-dependencies" "build-dependencies" ]);
        # transitive closure (incl. self) via a fixpoint over crateDirectDeps. `includeOptional` (default
        # true) is threaded to crateDirectDeps — seedCompilerClosure passes false (corpus-off seed build);
        # every other caller keeps the default (full closure incl. optional deps).
        crateClosure' = { includeOptional ? true }: start:
          let
            deps = crateDirectDeps { inherit includeOptional; };
            step = acc:
              let next = pkgs.lib.unique (acc ++ pkgs.lib.concatMap deps acc);
              in if builtins.length next == builtins.length acc then acc else step next;
          in pkgs.lib.sort (a: b: a < b) (step [ start ]);
        crateClosure = crateClosure' { };
        # `cargo -p C` LOADS the whole workspace (target auto-detection needs EVERY member to have a target
        # entry point present, else "no targets specified in the manifest") but only COMPILES C + its
        # dep-closure. So a per-crate check's fileset = FULL src/ for C's dep-CLOSURE (crateClosure — what
        # actually compiles) + ONLY the Cargo.toml of every non-closure member (NOT its src — see below) +
        # ONLY C's tests/. The buildPhase then MATERIALIZES a synthetic EMPTY target stub (src/lib.rs, +
        # src/main.rs for a [[bin]] member) for each non-closure member, so cargo can parse the workspace
        # WITHOUT the real src being in the fileset. This makes crateClosure LOAD-BEARING + delivers true
        # SRC-ISOLATION: editing an independent crate's src (outside C's closure) does NOT change C's
        # fileset → does NOT invalidate C's check (github-liaison #2134 review — allMemberSrc gave only
        # tests/-isolation; a real-src parse-floor still cross-triggered on the stubbed lib.rs edits, so the
        # stub must be SYNTHETIC/content-fixed, not the real file). No member has build.rs (verified).
        nonClosureManifests = excludeClosure:
          map (c: ./. + "/${rootWorkspaceCrates.${c}}/Cargo.toml")
            (builtins.filter (c: !(builtins.elem c excludeClosure)) rootCrateNames);
        # shell that writes an empty synthetic target stub for each non-closure member (lib.rs always; main.rs
        # if its Cargo.toml declares a [[bin]]/[bin]). Content-fixed → invariant to the real src.
        stubNonClosure = excludeClosure:
          let others = builtins.filter (c: !(builtins.elem c excludeClosure)) rootCrateNames; in
          pkgs.lib.concatMapStringsSep "\n"
            (c:
              let m = rootWorkspaceCrates.${c}; in ''
                mkdir -p "${m}/src"
                [ -f "${m}/src/lib.rs" ] || echo "" > "${m}/src/lib.rs"
                if grep -qE '^\[\[bin\]\]|^\[bin\]' "${m}/Cargo.toml"; then echo "fn main(){}" > "${m}/src/main.rs"; fi
              '')
            others;
        # the COMPILE inputs of a closure member: Cargo.toml + src/ (+ build.rs) — NOT its tests/ or
        # benches/. Scoping closure members to src/ (not the whole dir) means editing a DEPENDENCY crate's
        # tests/ does NOT invalidate a dependent's check (it never runs them) — github-liaison #2154: a
        # whole-dir closure member leaked its tests/ into the fileset. Only the UNDER-TEST crate's tests/
        # belongs in its check (added separately below).
        crateCompileSrc = c:
          let d = ./. + "/${rootWorkspaceCrates.${c}}"; in
          [ (d + "/Cargo.toml") ]
          ++ pkgs.lib.optional (builtins.pathExists (d + "/src")) (d + "/src")
          ++ pkgs.lib.optional (builtins.pathExists (d + "/build.rs")) (d + "/build.rs");
        # crane buildDepsOnly (operator fleet-velocity mandate — MR1, additive, NO consumer yet): compile the
        # workspace's DEPENDENCY closure ONCE into a content-addressed derivation cached in the /nix/store
        # (which the cache-nix-action rollout now shares across CI runs). Measured: deps are 61% of the
        # clippy/test compile (~74s of a 116s cold wall) — this caches that layer so a per-crate check (MR2/MR3
        # will consume it) recompiles only FIRST-PARTY src (~42s tail). depsOnlySrc = every member Cargo.toml +
        # root Cargo.toml/lock + synthetic stub srcs for ALL members (via `stubNonClosure []` — empty exclude
        # stubs everyone) so NO real first-party src is in the hash → a first-party edit does NOT invalidate
        # the dep cache. cargoVendorDir reuses seedCargoVendor (the offline root-lock vendor). This does NOT
        # touch the runtime-component build (REQUIRED_RUNTIME_HASH unaffected — that stays importCargoLock).
        cargoArtifactsSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (map (c: ./. + "/${rootWorkspaceCrates.${c}}/Cargo.toml") rootCrateNames)
            ++ [ ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]);
        };
        cargoArtifacts = craneLib.buildDepsOnly {
          pname = "cadenza-seed-deps";
          version = "0.0.0";
          src = cargoArtifactsSrc;
          cargoVendorDir = seedCargoVendor;
          # stub EVERY member so cargo can parse the workspace to compile deps, without any real first-party
          # src in the build (keeps the derivation hash invariant to first-party edits).
          # chmod first: fileset.toSource copies can be read-only, so the stub mkdir/echo would fail on the
          # tree without this (same guard the per-crate crane checks' preBuild uses before stubNonClosure).
          preBuild = ''
            chmod -R u+w .
            ${stubNonClosure [ ]}
          '';
          # doCheck = true (crane's DEFAULT — do NOT set false). #2282-review/v-ft crane measurement: with
          # doCheck=false, buildDepsOnly (a) sets cargoCheckExtraArgs="" instead of "--all-targets" so the dep
          # check never compiles test-target / dev-dependency artifacts, and (b) SKIPS its `cargo test --no-run`
          # check phase — crane's own comment on the default is "Run tests by default to ensure we cache any
          # dev-dependencies". So a doCheck=false cache holds ONLY the normal (non-dev) dep closure: clippy
          # restores it + wins (16→7-8m, it needs only those deps), but cargoTest must recompile the ENTIRE
          # dev-dep + test-harness closure from scratch on top of the first-party test build → it did MORE than
          # the old `cargo test`, reused LESS of the warm cache → test-ubuntu REGRESSED 16→23m. Letting doCheck
          # default to true warms the dev-dep/test-target layer too (a superset cache; clippy still restores its
          # subset + wins), so per-crate cargoTest reuses the warm deps like clippy does. Deps-only still: the
          # dummy-src `cargo test --no-run` compiles dev-deps but NO first-party tests (all members are stubbed).
          # Set EXPLICITLY (not just relying on the default) so the invariant is enforced in code — guards a
          # future crane-default flip or a comment-less copy of this block (Copilot #2288 review, LOW).
          doCheck = true;
        };
        # RELEASE dep-cache (v-nix, operator throughput 2026-08-09): the sibling of cargoArtifacts built at
        # --release. WHY: gate-check / codegen-check / bench-check run `cargo run --profile release -p xtask`
        # over a RAW vendor with NO crane cache, so ANY rotation (even a corpus-only .sexp edit) recompiles
        # the ENTIRE release dep closure + first-party from scratch (~330s measured on a 1-line .sexp edit;
        # deps are ~55 of the first ~62 crates compiled, the bulk). cargoArtifacts above is DEV-profile, so it
        # can't cache a release build. This caches the RELEASE dep closure ONCE (content-addressed, warm-kept
        # + shared across runs like cargoArtifacts) so those three checks recompile only first-party on a
        # rotation. Same buildDepsOnly shape (all-members-stubbed → hash invariant to first-party edits), just
        # CARGO_PROFILE=release. doCheck=false: these are `cargo run` (bin) checks, not tests — they need the
        # normal release dep closure, not the dev-dep/test-target layer, so skip the test-warm (smaller cache,
        # faster warm; the gate/codegen/bench run no test-targets).
        cargoArtifactsRelease = craneLib.buildDepsOnly {
          pname = "cadenza-seed-deps-release";
          version = "0.0.0";
          src = cargoArtifactsSrc;
          cargoVendorDir = seedCargoVendor;
          CARGO_PROFILE = "release";
          preBuild = ''
            chmod -R u+w .
            ${stubNonClosure [ ]}
          '';
          doCheck = false;
        };
        # RELEASE dep-cache built with the MERGED codegenVendor (root + cdz-runtime + cdz-nfc + build-std
        # locks) for the codegen/bench checks (v-nix, operator throughput 2026-08-09). gate-check uses the
        # seedCargoVendor release layer above (it only builds host binaries); codegen/bench need codegenVendor
        # (codegen builds cdz-runtime/cdz-nfc components via cargo-component; bench compiles against the
        # cdz-runtime lock), so they get their own release dep-cache over the same merged vendor. Same
        # buildDepsOnly shape; merged vendor via cargoVendorDir = codegenVendor (the symlinkJoin), matching
        # how the checks themselves source it (mkCargoVendorEnv { merged = true }).
        cargoArtifactsReleaseCodegen = craneLib.buildDepsOnly {
          pname = "cadenza-codegen-deps-release";
          version = "0.0.0";
          src = cargoArtifactsSrc;
          cargoVendorDir = codegenVendor;
          CARGO_PROFILE = "release";
          preBuild = ''
            chmod -R u+w .
            ${stubNonClosure [ ]}
          '';
          doCheck = false;
        };
        # crane MR2: per-crate CLIPPY via crane, consuming the shared cargoArtifacts (deps pre-compiled) so
        # only C's first-party src recompiles — NOT the whole dep closure every run (the ~14m→~6-7m win).
        #
        # craneCrateCommon: the SHARED per-crate crane inputs both the clippy + test makers compose with — a
        # per-crate isolation fileset + stub machinery (a crate's check invalidates only on its closure's src).
        # ONE home for these invariants (fileset scoping, chmod+stub preBuild, cargoArtifacts,
        # pinned vendor) so a future closure/stub tweak can't land in one maker but not the other — that
        # duplication was the ROOT of the earlier crane divergences (chmod #2262, --locked #2273; github-liaison
        # #2279). craneLib is already toolchain-pinned (overrideToolchain). preBuild chmod's the (read-only)
        # fileset.toSource copy + stubs the non-closure members so `cargo -p C` parses the workspace (crane
        # restores cargoArtifacts' target/ before this).
        craneCrateCommon = { crate, extraSrc ? [ ], extraInputs ? [ ] }:
          let closure = crateClosure crate; in
          {
            version = "0.0.0";
            inherit cargoArtifacts;
            cargoVendorDir = seedCargoVendor;
            # CDZ_RUN_TIMEOUT_SECS=300 (default 30): cdz-run arms a WALL-CLOCK epoch deadline (a background
            # thread bumps the wasmtime engine epoch every 100ms regardless of guest CPU — cdz-run arm_epoch_ticker).
            # Under heavy parallel cargo test, a correct + terminating rcdzc match_engine constant-stack-loop test
            # (a_tail_recursive_list_fold / a_non_tail_list_fold / a_tail_recursive_sum_consumer) gets starved
            # off-core past the 30s WALL bound and traps `interrupt` → a false-red MERGE gate (148-172s loaded vs
            # 76-98s unloaded; 3/0 isolated). xtask's `test` step already sets 300 (main.rs), but the nix localGate
            # runs the per-crate craneLib.cargoTest checks, not that step — so it hit the 30s default. Set here on
            # the SHARED base so every per-crate test derivation gets it at once (clippy ignores it). 300s still
            # catches a genuine infinite loop (blows 300s wall too); harness-only, prod/CI cdz-run keeps 30s.
            # Cross-owner fold: v-fleet-tooling owns gate policy + the sibling xtask fix, v-nix owns the flake.
            CDZ_RUN_TIMEOUT_SECS = "300";
            # RUST_MIN_STACK=64M — the SIBLING of the timeout env (xtask's test step sets both; main.rs). A
            # deep-but-finite recursion test running on libtest's own ~2MB worker thread (NOT wrapped in rcdzc's
            # explicit-stack host::run_with_compiler_stack) nondeterministically SIGABRTs on stack overflow under
            # fleet build load — a recurring false-red class point-fixed across v-diagnostics/v-syntax/v-runtime.
            # Some rcdzc tests EXPLICITLY depend on the floor (tests.rs comments "RUST_MIN_STACK=64M passes":
            # they terminate but are deep). Only benchCheck set it before; the per-crate cargoTest path ran at
            # the ~2MB default → the merge gate could still SIGABRT-false-red under load, parallel to the timeout
            # hole. v-ft diffed the full xtask-test-step env vs the nix path: this + the timeout are the ENTIRE
            # delta, so this closes the whole xtask-env-not-mirrored-onto-nix-per-crate-cargoTest class.
            RUST_MIN_STACK = "67108864";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions (
                (pkgs.lib.concatMap crateCompileSrc closure)
                ++ nonClosureManifests closure
                ++ pkgs.lib.optional
                  (builtins.pathExists (./. + "/${rootWorkspaceCrates.${crate}}/tests"))
                  (./. + "/${rootWorkspaceCrates.${crate}}/tests")
                ++ [ ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]
                ++ extraSrc);
            };
            nativeBuildInputs = extraInputs;
            preBuild = ''
              chmod -R u+w .
              ${stubNonClosure closure}
            '';
            doInstallCargoArtifacts = false;
          };
        # per-crate CLIPPY via crane. cargoClippyExtraArgs is `cargo clippy -p C
        # --all-targets -- -D warnings`; crane's cargoClippy INJECTS --locked (do NOT add it — a 2nd errors
        # "cannot be used multiple times", #2273). Strict pattern (like craneCrateCommon) so a typo'd key is
        # caught HERE at the call-contract, not late/silently inside the helper (github-liaison #2282); @args
        # still forwards the full attrset to craneCrateCommon — no behavior change, just the strict interface back.
        mkCrateClippyCrane = { crate, extraSrc ? [ ], extraInputs ? [ ] }@args:
          craneLib.cargoClippy ((craneCrateCommon args) // {
            pname = "cargo-clippy-${crate}";
            cargoClippyExtraArgs = "-p ${crate} --all-targets -- -D warnings";
          });
        # per-crate TEST via crane, consuming the shared cargoArtifacts (deps + dev-dep/test-target layer
        # pre-compiled, since cargoArtifacts is doCheck=true) so a crate's test rerun recompiles only ITS
        # closure's first-party src + runs only ITS test binaries — NOT the whole workspace.
        #
        # THROUGHPUT REVIVAL (v-nix, operator 1-min-gate mandate 2026-08-09): a per-crate test maker existed
        # before and was reverted to a whole-workspace `cargo test --workspace` (option-b) because under the
        # OLD monolithic GHA model crane cargoTest with doCheck=FALSE deps recompiled the dev-dep/test-target
        # closure per crate (~18-19m vs ~16m cargo baseline). TWO things changed: (1) cargoArtifacts is now
        # doCheck=TRUE, so the dev-dep + test-target layer is ALREADY warm in the shared cache — the exact
        # recompile that caused the old regression is gone; (2) the gate is now PER-MR (GHA-off cutover), so
        # the whole-workspace `cargo test --workspace` RE-RUNS EVERY test on ANY one-crate edit (diagnosed via
        # drv-hash probe: testCheck rotates on any first-party change) — that per-MR re-run is the ~25min
        # stall the operator is calling out ("a small change should take a minute"). Per-crate cargoTest makes
        # a one-crate edit rerun only that crate + dependents' tests, cache-hitting the rest — the same win
        # the per-crate clippy shards already deliver. Coverage parity (∪ per-crate == workspace) is asserted
        # by testCrateCoverageAssert below (mirrors crateClosureAssert), so no test is silently dropped.
        mkCrateTestCrane = { crate, extraSrc ? [ ], extraInputs ? [ ] }@args:
          craneLib.cargoTest ((craneCrateCommon args) // {
            pname = "cargo-test-${crate}";
            # crane's cargoTest injects --locked; cargoTestExtraArgs scopes it to this crate's own tests.
            cargoTestExtraArgs = "-p ${crate}";
          });
        # CLOSURE guard (concierge mandate): pure-eval assert that the fromTOML walk yields the EXPECTED
        # closures for anchor crates — a Cargo.toml restructure that breaks the walk fails LOUD (throws at
        # eval → fails `nix flake check`) rather than silently under-scoping a crate's inputs.
        crateClosureAssert =
          let
            expected = {
              # cdz-run gained a `cdz-contract` dep in the base62 flip (#3090: content_address delegates
              # to cdz_contract::Hash::of), so cdz-contract enters both the rcdzc closure (via its cdz-run
              # dev-dep) and the xtask closure (direct dep). cadenza-ast was already present via cdz-contract.
              # cdz-run then gained a `cdz-corpus-grade` dep (#3470: the shared corpus grade compare), which
              # only path-deps cadenza-syntax (already present) — so it enters rcdzc's closure via cdz-run.
              rcdzc = [ "cadenza-ast" "cadenza-syntax" "cdz-contract" "cdz-corpus-grade" "cdz-num" "cdz-rt" "cdz-run" "rcdzc" ];
              cadenza-syntax = [ "cadenza-ast" "cadenza-syntax" ];
              cdz-num = [ "cdz-num" ];
              # cdz-world-artifact deps only cadenza-ast (the language's binary-AST builders/codec) + the
              # external wit-parser; xtask still deps cadenza-ast via codegen.rs, so its closure is unchanged.
              cdz-world-artifact = [ "cadenza-ast" "cdz-world-artifact" ];
              xtask = [ "cadenza-ast" "cdz-contract" "cdz-rust-render" "xtask" ];
            };
            mismatches = builtins.filter (n: (crateClosure n) != expected.${n})
              (builtins.attrNames expected);
          in
          if mismatches != [ ] then
            throw ("flake.nix Part-B closure-assert: fromTOML closure walk disagrees with expected for "
              + builtins.toString mismatches
              + " — the crate dep graph changed; re-verify vs `cargo metadata` and update `expected`.")
          else
            pkgs.runCommand "crate-closure-assert" { } ''
              echo "ok: per-crate closures match expected (${builtins.toString (builtins.attrNames expected)})" > $out
            '';

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
            # After v-cdz-delegate's #3397, a `--no-default-features` cdz (which `seedCompiler` is)
            # DELEGATES compilation to the external `cdz-compile` CLI instead of linking rcdzc — so this
            # cdz needs `cdz-compile` reachable. Point at the SEPARATE content-addressed `cdzCompile`
            # derivation (kept independent of cdz — the whole caching point; per the agreed seam). The
            # delegate resolves `CDZ_COMPILE_BIN` first. Harmless BEFORE #3397 (cdz is still in-process, so
            # this var sits unused) — landing it here keeps the harness/project builds green when #3397 flips.
            CDZ_COMPILE_BIN = "${cdzCompile}/bin/cdz-compile";
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
            # After v-cdz-delegate's #3397, a `--no-default-features` cdz (which `seedCompiler` is)
            # DELEGATES compilation to the external `cdz-compile` CLI instead of linking rcdzc — so this
            # cdz needs `cdz-compile` reachable. Point at the SEPARATE content-addressed `cdzCompile`
            # derivation (kept independent of cdz — the whole caching point; per the agreed seam). The
            # delegate resolves `CDZ_COMPILE_BIN` first. Harmless BEFORE #3397 (cdz is still in-process, so
            # this var sits unused) — landing it here keeps the harness/project builds green when #3397 flips.
            CDZ_COMPILE_BIN = "${cdzCompile}/bin/cdz-compile";
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


        # Full-CI-in-nix increment 6e: the GHA `cad-tests` job — `cdz test` on the 4 committed
        # in-tree Cadenza PROJECTS (implementation/{cad,compiler-ml,choreography,iterators}). These are
        # pure-Cadenza (Project.cdz + src/*.cdz), NOT the excluded Rust cdz-cad crate — so no cmake/C++,
        # just the S3 testCadenzaProject pattern applied to real project dirs: the nix-built seedCompiler
        # runs each project's @test suite, resolving the value-heap runtime from my componentStore
        # (CDZ_STORE) — skipping the CI job's `xtask build` + native cdz rebuild.
        #
        # PER-PROJECT SPLIT (v-nix, operator+concierge test-throughput arc 2026-08-08, approved as slice
        # (b)): the old form gave ALL 4 projects ONE union `src`, so a one-line edit to ANY project busted
        # the whole derivation and reran all 4 (~35m). Each project is self-contained (`modules =
        # ["src/*.cdz"]`, no cross-dir imports), so this splits into one `cdz test` derivation PER project,
        # each with a NARROW fileset (just its own dir). A change to one project now only busts that one
        # derivation — the other 3 cache-hit — for a ~4x win on the common single-project-change case.
        # Mirrors the per-crate clippy shard pattern (each shard its own narrow closure, an aggregate over
        # all). The `cad-tests` check below is now an AGGREGATE that depends on all 4 (required-context name
        # unchanged → no ruleset edit); the 4 per-project derivations are ALSO exposed individually as
        # checks.<sys>.cad-test-{cad,compiler-ml,choreography,iterators} so a candidate touching one project
        # can build just that one, and cache-warm roots them the same way.
        mkCadProjectTest = { name, dir }: pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-cad-test-${name}";
          version = "0.0.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = dir;
          };
          nativeBuildInputs = [ seedCompiler ];
          # cdz-compile reachable for post-#3397 delegation (see buildCadenzaProject); harmless before it.
          CDZ_COMPILE_BIN = "${cdzCompile}/bin/cdz-compile";
          buildPhase = ''
            runHook preBuild
            set -o pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            # Run this project's @test suite, resolving the runtime from the nix store. A non-zero
            # `cdz test` propagates (pipefail) and fails the build.
            echo "== cdz test ${name} =="
            cdz test "implementation/${name}" | tee "$TMPDIR/cad-test.out"
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp "$TMPDIR/cad-test.out" "$out"
            runHook postInstall
          '';
        };
        cdzCadProjectTests = {
          cad-test-cad = mkCadProjectTest { name = "cad"; dir = ./implementation/cad; };
          cad-test-compiler-ml = mkCadProjectTest { name = "compiler-ml"; dir = ./implementation/compiler-ml; };
          cad-test-choreography = mkCadProjectTest { name = "choreography"; dir = ./implementation/choreography; };
          cad-test-iterators = mkCadProjectTest { name = "iterators"; dir = ./implementation/iterators; };
        };
        # AGGREGATE over the 4 per-project tests — the required `cad-tests` context. A change to one project
        # rebuilds only that project's derivation; the aggregate re-links (cheap runCommand) and the other 3
        # cache-hit. Advisory-by-omission → unilateral cargo-twin retire once green.
        cdzCadTestsCheck = pkgs.runCommand "cdz-cad-tests" cdzCadProjectTests ''
          echo "ok: cad-tests aggregate — cdz test on cad + compiler-ml + choreography + iterators (per-project split)" > "$out"
        '';

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
            ${mkCargoVendorEnv { vendor = rcdzcWasmVendor; }}
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

        # Full-CI-in-nix increment 3: the NATIVE half of the GHA `rcdzc-wasm` job (cargo test + clippy +
        # fmt in the rcdzc-wasm crate dir). The job's OTHER half — the wasm32-wasip1 build — is already
        # the `rcdzcWasm` derivation above, so `nix flake check` covers the whole job via two checks. This
        # reuses rcdzc-wasm's OWN vendor + 7-crate src (it's a standalone workspace, its own Cargo.lock).
        # Advisory-by-omission job (not in ruleset 10560470) → once green I can retire its cargo twin
        # UNILATERALLY, no lockstep.
        rcdzcWasmNativeCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "rcdzc-wasm-native";
          version = "0.0.0";
          src = rcdzcWasmSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = rcdzcWasmVendor; }}
            cd implementation/seed/crates/rcdzc-wasm
            cargo test --locked
            cargo clippy --all-targets --locked -- -D warnings
            cargo fmt --check
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: rcdzc-wasm native (test + clippy + fmt)" > "$out"
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
        # FALLS OUT as the platform content address of that output (`hashOf`), exposed via `packages.*-hash`.
        # This runtime derivation pins no content-address literal — `runtime_abi.rs`'s recorded hash becomes
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
            # cdz-runtime `#[path]`-includes the cadenza-ast codec-core (ast/leb128/codec) from the sibling
            # rcdzc crate — the shared canonical serializer the `ast-encode`/`ast-decode` heap ops reuse so
            # the runtime bytes are byte-identical to the compile-time `Ast.encode` fold (copy-don't-depend
            # via shared SOURCE, NOT a crate dep — the #459 cross-crate-LTO/frozen-hash lesson). Those three
            # files ARE part of the runtime's source, so they must be staged into this tightly-scoped build
            # sandbox or the relative `../../rcdzc/src/*.rs` include fails "No such file or directory". A
            # change to any of the three correctly rotates the runtime component (its bytes depend on them).
            ./implementation/seed/crates/rcdzc/src/ast.rs
            ./implementation/seed/crates/rcdzc/src/codec.rs
            ./implementation/seed/crates/rcdzc/src/leb128.rs
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
        #   emitRaw  : also expose the RAW pre-strip wasm as a second output `raw` (R3, v-nix+v-runtime
        #              2026-08-09). WHY: `xtask codegen` reads the `cdz-abi` CUSTOM SECTION (read_abi_imm_unit
        #              → the 4-byte CDZ_ABI_IMM_UNIT) from the RAW build BEFORE `wasm-tools strip -a` removes
        #              all custom sections — then hashes the STRIPPED bytes. R3 inverts codegen to CONSUME the
        #              nix runtime instead of self-building it a 3rd time (on top of this derivation + the
        #              gate build), so the derivation must expose BOTH: `out` (stripped = the hashed artifact,
        #              UNCHANGED) + `raw` (pre-strip, cdz-abi intact) from ONE build (multi-output, not a 2nd
        #              derivation — that would rebuild + defeat the dedup). `out` stays byte-identical, so
        #              adding `raw` is a true no-op on REQUIRED_RUNTIME_HASH. Only the runtime sets emitRaw
        #              (nfc's hash is content_address of the stripped nfc; it reads no custom section).
        # `stampNfcHash` (default null): the hash-file of the NFC component (a `hashOf` derivation, e.g.
        # `nfcHash`) whose content address is stamped INLINE into this component's `cadenza:nfc/normalize`
        # import before the strip — turning the bare import into the self-describing
        # `cadenza:nfc/normalize@0.0.0+<hash>` so a runtime resolves NFC purely from the import (no
        # runtime.toml/mapping; operator directive 2026-08-23). Only the value-heap runtimes set it
        # (`mkRuntime` passes `nfcHash`); the NFC component + guests leave it null. The hash is `cat` from the
        # SHARED `hashOf` derivation (no IFD; computed once, not re-run here) and stamped by the
        # `cdz-component-rewrite` CLI, mirroring `xtask build`'s `stamp_nfc_into_heap` so nix and the
        # self-build agree byte-for-byte.
        mkStripComponent = { pname, crateDir, artifact, src, vendor, features ? [ ], emitRaw ? false, stampNfcHash ? null }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";
            outputs = if emitRaw then [ "out" "raw" ] else [ "out" ];

            nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ]
              ++ pkgs.lib.optionals (stampNfcHash != null) [ cdzComponentRewrite ];

            featuresArg = pkgs.lib.optionalString (features != [ ])
              ("--features " + pkgs.lib.concatStringsSep "," features);

            buildPhase = ''
              runHook preBuild
              export RUSTC_BOOTSTRAP=1
              # Merged vendor (crates.io + build-std + the NFC component-dep lock) → merged = true.
              ${mkCargoVendorEnv { inherit vendor; merged = true; }}
              cd implementation/seed/crates/${crateDir}
              # --locked honors the committed Cargo.lock exactly. Network is blocked by CARGO_NET_OFFLINE
              # (set by mkCargoVendorEnv) + the sandbox itself — NOT the `--offline` FLAG: the runtime's
              # world imports the NFC component (a `[package.metadata.component.target.dependencies]` WIT
              # path-dep on ../cdz-nfc/wit, FINDING#23), and the `--offline` flag makes `cargo component`
              # refuse that component-dep resolution outright ("lock file must be provided when offline
              # mode is enabled") even though it's a LOCAL path needing no network. CARGO_NET_OFFLINE
              # blocks the crates.io registry (our vendor covers it) while still letting cargo-component
              # resolve the local WIT dep. A truly-missing dep still fails LOUD (no network in the sandbox).
              cargo component build --release --target wasm32-unknown-unknown --locked $featuresArg
              runHook postBuild
            '';

            # CANONICALIZE (strip the tool-version producers sections) — the same step xtask's
            # canonicalize_runtime does. The stripped bytes are the content-addressed artifact ($out). When
            # emitRaw, ALSO copy the pre-strip wasm to $raw (cdz-abi custom section intact) BEFORE stripping,
            # so an R3 consumer reads cdz-abi from $raw + hashes $out — codegen's read-raw-hash-stripped shape.
            installPhase = ''
              runHook preInstall
              ${pkgs.lib.optionalString emitRaw ''
                cp target/wasm32-unknown-unknown/release/${artifact}.wasm "$raw"
              ''}
              ${if stampNfcHash != null then ''
                # STAMP the NFC address inline into the heap's `cadenza:nfc/normalize` import BEFORE strip
                # (strip -a removes any `producers` the re-encode adds; the stamped+stripped bytes are the
                # content-addressed artifact). The address is the NFC component's content address, `cat` from
                # the SHARED `hashOf` derivation (`nfcHash`) — the same value the store keys by — so the
                # stamped `+hash` matches `<store>/<hash>.wasm`. `$out` = stamped+stripped; `$raw` (above)
                # stays the pre-stamp build (cdz-abi intact for codegen). Mirrors `xtask build`'s `stamp_nfc_into_heap`.
                nfc_hash=$(cat ${stampNfcHash})
                cdz-component-rewrite \
                  target/wasm32-unknown-unknown/release/${artifact}.wasm \
                  target/wasm32-unknown-unknown/release/${artifact}.nfc-stamped.wasm \
                  "cadenza:nfc/normalize=0.0.0+$nfc_hash"
                wasm-tools strip -a \
                  target/wasm32-unknown-unknown/release/${artifact}.nfc-stamped.wasm \
                  -o "$out"
              '' else ''
                wasm-tools strip -a \
                  target/wasm32-unknown-unknown/release/${artifact}.wasm \
                  -o "$out"
              ''}
              runHook postInstall
            '';
          };

        # The value-heap runtime derivations bind mkStripComponent to the cdz-runtime crate. `stampNfcHash =
        # nfcHash` stamps the NFC component's content address inline into the heap's `cadenza:nfc/normalize`
        # import (self-describing dep — no runtime.toml/mapping; operator directive 2026-08-23). `nfcHash` is
        # `hashOf nfc` and `nfc` is itself a plain (unstamped) mkStripComponent, so there is no cycle.
        mkRuntime = { pname, features, emitRaw ? false }:
          mkStripComponent {
            inherit pname features emitRaw;
            crateDir = "cdz-runtime";
            artifact = "cdz_runtime";
            src = runtimeSrc;
            vendor = runtimeVendor;
            stampNfcHash = nfcHash;
          };

        # The RELEASE runtime — what a shipped program pins (REQUIRED_RUNTIME_HASH). emitRaw = true adds the
        # `raw` output (pre-strip wasm, cdz-abi intact) for R3's codegen-consumer; the default `out` (stripped
        # = the hashed artifact) is byte-unchanged, so this is a no-op on REQUIRED_RUNTIME_HASH.
        runtime = mkRuntime {
          pname = "cdz-runtime-component";
          features = [ ];
          emitRaw = true;
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

        # ── a Cadenza `.cdz` reducer GUEST → wasm component (operator directive 2026-08-23) ─────────
        #
        # Compile a Cadenza reducer SOURCE (`.cdz`) to a reducer-world wasm component. A Cadenza-authored guest
        # fills every `harnessPrograms` slot now — the interim hand-written Rust `reducer-echo` fixtures it
        # replaced have been retired (the operator's "Cadenza guests, no more Rust" priority, §9). Uses
        # `cdz compile <src> --target wasm` — the `.cdz` declares its reducer world INLINE via `(world …)`
        # (the external `KIND_WIT_WORLD` artifact path is a not-yet-wired follow-up, rcdzc `wit_world.rs`).
        # Canonicalized (`strip -a`) + content-addressed, so it registers identically to any component.
        #
        # ⚠ GAP-ASSESSMENT STATUS (2026-08-23, coordinated with v-platform who authors the probe `.cdz`):
        # the reducer-world compile path is UNEXERCISED end-to-end — no `.cdz` anywhere yet declares a
        # `(world …)`, and a reducer that CALLS a compound-result host import (state.get/blobs.get) needs
        # v-rust-backend's shared-ALLOCATOR slice (not yet landed). A MINIMAL echo reducer (exports
        # on-message → step, no host-import calls) is the first thing to try. This function is READY; the
        # `harnessPrograms` entry + a `packages.<name>` land the moment v-platform's probe `.cdz` compiles.
        # `componentName` (the reducer-world export interface, e.g. `cadenza:platform/guest`) is passed to
        # `cdz compile --component-name` so rcdzc publishes the guest's exports under that fully-qualified WIT
        # interface — the name v-platform's host `WasmReducer` binds (`run_reducer_typed(bytes,
        # "cadenza:platform/guest", "on-message", …)`). Without it rcdzc has no `component_name` and falls
        # through to the fixed-scalar-param export path, declining a record-of-bytes `message` param (the
        # empirical gap v-nix hit + v-rb diagnosed 2026-08-23; #3117 also derives it from an FQ in-source
        # export, but passing it explicitly here is encoding-agnostic to how the probe declares its export).
        # `witWorld` (default null): the path to a `KIND_WIT_WORLD` binary-AST artifact (produced by
        # `cargo xtask world-artifact` = `worldArtifacts` below) declaring the reducer's world — its imports
        # AND its typed guest export — from the shared `cdz-platform/wit/world.wit` (single source of truth).
        # Passed as the `wit-world:reducer-world=<bin>` compile input. A reducer that only ECHOES (no host
        # import) can type its export with a small INLINE `(world …)` and needs no artifact; a reducer that
        # CALLS a host import (`state`/`identity`/`blobs`) needs the world's IMPORT declarations, which the
        # external artifact provides (the inline form declares only the export). `witWorldName` names which
        # world in the artifact to bind (the platform reducer world).
        # Compile a Cadenza guest to a canonicalized wasm component. Single-file by default; a MULTI-FILE
        # package (the §9 checker guest imports library modules — contracts/check.cdz, verdict.cdz,
        # guests/log-schema.cdz — via `import { … } from "<stem>"`) passes its extra sources in `libs`.
        #
        # WHY `libs` are COPIED to clean basenames (not passed as store paths): `cdz` resolves an
        # `import "<name>"` against the input's ARTIFACT NAME, which for a bare source path is the file STEM.
        # A nix store path is hash-prefixed (`/nix/store/<hash>-check.cdz`), so its stem is `<hash>-check` —
        # `import { … } from "check"` would fail `CDZ0201: names unknown package file`. A source input cannot
        # be given an explicit name either (`name=path` is AST-only — decodes source as binary AST and fails;
        # there is no `source:`/`cdz:` kind prefix). So the ONLY way is to `cp` each source to its clean
        # basename in the build dir and compile the bare clean paths, whose stems then match the imports.
        # (v-nix verified this empirically 2026-08-24: hash-prefixed → CDZ0201; cp-to-clean-name → compiles.)
        # `--entry` names the boundary file by its clean stem; required once >1 source is given.
        mkCadenzaGuest = { pname, src, componentName ? null, entry ? null, libs ? [ ], witWorld ? null, witWorldName ? "reducer-world" }:
          let
            flags = pkgs.lib.concatStringsSep " " (builtins.filter (s: s != "") [
              (pkgs.lib.optionalString (witWorld != null) "wit-world:${witWorldName}=${witWorld}")
              (pkgs.lib.optionalString (componentName != null) "--component-name ${componentName}")
              (pkgs.lib.optionalString (entry != null) "--entry ${entry}")
            ]);
            # Single-file: compile the store path directly (its mangled stem is irrelevant with no imports).
            # Multi-file: stage every source under its clean basename, then compile the bare clean paths.
            compile =
              if libs == [ ] then
                "cdz compile ${src} --target wasm -o guest.wasm ${flags}"
              else
                pkgs.lib.concatStringsSep "\n" (
                  [ "cp ${src} ${baseNameOf src}" ]
                  ++ map (l: "cp ${l} ${baseNameOf l}") libs
                  ++ [
                    ("cdz compile ${baseNameOf src} ${pkgs.lib.concatMapStringsSep " " baseNameOf libs} "
                      + "--target wasm -o guest.wasm ${flags}")
                  ]
                );
          in
          pkgs.runCommand pname
            {
              nativeBuildInputs = [ seedCompiler pkgs.wasm-tools ];
              # cdz-compile reachable for post-#3397 delegation (see buildCadenzaProject); harmless before it.
              # This is the HARNESS guest build — the conformance suite depends on it, so it must not break
              # the moment #3397 flips cdz to delegating.
              CDZ_COMPILE_BIN = "${cdzCompile}/bin/cdz-compile";
            } ''
            set -euo pipefail
            ${compile}
            # Canonicalize (strip the tool-version `producers` sections) so the guest's content address is
            # reproducible cross-host, exactly like the Rust guests (mkStripComponent) + the runtime.
            wasm-tools strip -a guest.wasm -o "$out"
          '';

        # `worldArtifacts`: the platform reducer worlds as `KIND_WIT_WORLD` binary artifacts, generated ONCE
        # from `cdz-platform/wit/world.wit` (single source of truth) by the isolated `cdz-world-artifact`
        # utility CLI, for a host-import-calling Cadenza guest to consume via `mkCadenzaGuest`'s `witWorld`.
        # Built like `contractHasher` (plain cargo over the offline root vendor, `-p cdz-world-artifact`);
        # reuses `platformItestSrc` (has the crate src + the wit). The utility emits one `<world>.bin` per
        # world the document declares (no world name is baked into the tool), so `$out` gets `reducer-world.bin`
        # + `event-reducer-world.bin`. This shells out to the small utility directly — no xtask in the path
        # (operator directive 2026-08-24: decompose xtask into small single-purpose utility programs).
        worldArtifacts = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-platform-world-artifacts";
          version = "0.0.0";
          src = platformItestSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
            cargo build --release --locked -p cdz-world-artifact --bin cdz-world-artifact
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            ./target/release/cdz-world-artifact \
              implementation/seed/crates/cdz-platform/wit/world.wit "$out"
            # The TEST-ONLY arg-probe world (its OWN package `cadenza:test-arg-probe`, in wit/test/), for the
            # arg-VALUE-capture conformance gate. It imports `arg-probe` and exports `cadenza:platform/guest`,
            # so it needs the platform package resolved cross-package — passed as `--dep wit/world.wit` (the
            # `Worlds::parse_with_deps` path, #3424). Emitted into the SAME store as the platform worlds, so
            # `cadenzaWorldArgs "arg-probe-world"` resolves it and a `guests/arg-probe-world/<guest>/` dir
            # auto-discovers against it (v-platform-itest builds that guest + the arg-value checker).
            ./target/release/cdz-world-artifact \
              implementation/seed/crates/cdz-platform/wit/test/arg-probe.wit "$out" arg-probe-world \
              --dep implementation/seed/crates/cdz-platform/wit/world.wit
            runHook postInstall
          '';
        };

        # AUTO-ENUMERATED Cadenza reducer guests (operator 2026-08-24 — zero hardcoded reducer/world names):
        # the guests are a two-level tree `guests/<world>/<reducer>/reducer.cdz`, where the PARENT directory
        # NAMES THE WORLD the reducer targets. `readDir` derives BOTH the worlds AND the reducer names from
        # the tree — adding a guest is dropping a directory, nothing here changes.
        #   - `guests/inline/<reducer>/`            → the reducer declares its world INLINE in `reducer.cdz`
        #                                             (echoes; no host imports, so no external artifact needed).
        #   - `guests/reducer-world/<reducer>/`     → compile against `worldArtifacts`'s `reducer-world.bin`
        #                                             (ordinary world) — for a guest that CALLS a host import
        #                                             (its inline export can't declare imports).
        #   - `guests/event-reducer-world/<reducer>/` → against `event-reducer-world.bin` (privileged world).
        # A top-level dir whose subdirs hold no `reducer.cdz` is naturally skipped. Each guest is keyed in
        # `harnessPrograms` by its `<reducer>` dir name.
        cadenzaGuestsDir = ./implementation/seed/crates/cdz-platform/guests;
        # world → the mkCadenzaGuest witWorld args ("inline" ⇒ none; else the KIND_WIT_WORLD artifact).
        cadenzaWorldArgs = world:
          pkgs.lib.optionalAttrs (world != "inline") {
            witWorld = "${worldArtifacts}/${world}.bin";
            witWorldName = world;
          };
        # ── transitive lib auto-resolution (a contract's own imports flow into a dependent guest's lib set) ──
        # A guest's `libs` manifest lists the modules it DIRECTLY imports, but those modules import further
        # modules (e.g. a contract's `descriptor` imports `contract-id`). Rather than every manifest re-listing
        # each transitive lib — churn that grows as descriptor-on-every-contract spreads `import … from
        # "contract-id"` into every contract — we CLOSE the manifest over the `import { … } from "<stem>"`
        # graph. Over-inclusion is safe (an unused staged source only adds a file; only UNDER-inclusion breaks
        # the compile, CDZ0201), so the parse is deliberately liberal and the closure only ever ADDS libs.
        #
        # stem (module basename sans `.cdz`, the name an `import … from "<stem>"` resolves) → repo-relative
        # source path, over every contract + guest library module.
        libStemToPath =
          let
            # Every *.cdz UNDER a dir, RECURSING into subdirs — contracts are split across
            # contracts/kernel/ + contracts/userspace/, so a contract lives at contracts/<sub>/<x>.cdz. This is
            # layout-agnostic: on a flat dir it just finds the top-level files; on a nested one it descends.
            recurseCdz = d: builtins.concatMap
              (name:
                let sub = "${d}/${name}"; in
                if (builtins.readDir (./. + "/${d}")).${name} == "directory" then recurseCdz sub
                else if pkgs.lib.hasSuffix ".cdz" name
                then [ { name = pkgs.lib.removeSuffix ".cdz" name; value = sub; } ]
                else [ ])
              (builtins.attrNames (builtins.readDir (./. + "/${d}")));
            # Only the TOP-LEVEL *.cdz of a dir — for guests/, whose shared library modules (reducer-lib,
            # checker-lib, log-schema, contract-id) sit at the top level while its subdirs are guest PACKAGES
            # (each a reducer.cdz), which are NOT importable libs and must not enter the stem registry.
            topCdz = d: map
              (f: { name = pkgs.lib.removeSuffix ".cdz" f; value = "${d}/${f}"; })
              (builtins.filter
                (f: pkgs.lib.hasSuffix ".cdz" f
                  && (builtins.readDir (./. + "/${d}")).${f} == "regular")
                (builtins.attrNames (builtins.readDir (./. + "/${d}"))));
          in
          builtins.listToAttrs (
            recurseCdz "implementation/seed/crates/cdz-platform/contracts"
            ++ topCdz "implementation/seed/crates/cdz-platform/guests");
        # The library stems a source file imports — every `from "<stem>"` occurrence (liberal: a match not in
        # libStemToPath, e.g. a stray docstring mention, is filtered out by the closure operator below).
        importStemsOf = relPath:
          let
            content = builtins.readFile (./. + "/${relPath}");
            tails = pkgs.lib.drop 1 (pkgs.lib.splitString "from \"" content);
          in
          map (seg: builtins.head (pkgs.lib.splitString "\"" seg)) tails;
        # Close a manifest's DIRECT lib paths over the import graph → the full transitive set of source paths.
        closeLibs = relPaths:
          map (e: ./. + "/${e.key}") (builtins.genericClosure {
            startSet = map (p: { key = p; }) relPaths;
            operator = { key }: map (stem: { key = libStemToPath.${stem}; })
              (builtins.filter (stem: libStemToPath ? ${stem}) (importStemsOf key));
          });

        # { <reducer-dir-name> = <compiled guest>; } over every guests/<world>/<reducer>/reducer.cdz.
        cadenzaGuests = builtins.foldl'
          (acc: world:
            let
              worldDir = cadenzaGuestsDir + "/${world}";
              reducers = builtins.filter
                (r: builtins.pathExists (worldDir + "/${r}/reducer.cdz"))
                (builtins.attrNames (builtins.readDir worldDir));
            in
            acc // builtins.listToAttrs (map
              (r:
                let
                  guestDir = worldDir + "/${r}";
                  # A MULTI-FILE guest carries a `libs` manifest beside its reducer.cdz: one repo-relative
                  # source path per line (the library modules it imports). Absent ⇒ single-file (unchanged).
                  # The manifest lives WITH the guest, so no reducer/lib names are hardcoded in the flake.
                  libsFile = guestDir + "/libs";
                  libLines = pkgs.lib.optionals (builtins.pathExists libsFile)
                    (builtins.filter (s: s != "")
                      (pkgs.lib.splitString "\n" (builtins.readFile libsFile)));
                  # entry names the boundary file by its clean stem; required once libs add >1 source.
                  multiFileArgs = pkgs.lib.optionalAttrs (libLines != [ ]) {
                    # Transitively close the manifest over the import graph (a listed contract pulls in the
                    # libs IT imports, e.g. contract-id), so a manifest need only list its DIRECT imports.
                    libs = closeLibs libLines;
                    entry = pkgs.lib.removeSuffix ".cdz" (baseNameOf (guestDir + "/reducer.cdz"));
                  };
                in
                {
                  name = r;
                  value = mkCadenzaGuest ({
                    pname = "cdz-platform-${r}-component";
                    src = guestDir + "/reducer.cdz";
                    componentName = "cadenza:platform/guest";
                  } // cadenzaWorldArgs world // multiFileArgs);
                })
              reducers))
          { }
          # top-level entries that are directories (candidate world dirs; non-world dirs contribute nothing).
          (builtins.filter (w: (builtins.readDir cadenzaGuestsDir).${w} == "directory")
            (builtins.attrNames (builtins.readDir cadenzaGuestsDir)));

        # ── the integration-test HARNESS framework (design/cadenza-platform.md §9) ─────────────────
        #
        # The shape the operator asked for (PR #2994 review): a directory of PROGRAMS compiled once into a
        # wasm store (by NAME), a directory of HARNESS RUNS (each an s-expr describing a whole run), and ONE
        # derivation PER run so caching is fine-grained — changing a run reruns only that run, changing a
        # program reruns only the runs that USE it, and neither ever rebuilds the integration-test binary.
        #
        # `harnessPrograms`: the wasm store, name → the reproducibly-built component. A run refers to a
        # program by name; `mkHarnessRun` resolves the name to this store path. EVERY program is an
        # auto-enumerated Cadenza guest (keyed by its `guests/` dir name, e.g. `reducer-echo-cdz`,
        # `reducer-identity-cdz`, `reducer-provenance-cdz`), so a new guest needs no edit here (see
        # `cadenzaGuests`). The interim Rust `reducer-echo` / `reducer-echo-check` fixtures were retired once
        # the Cadenza pipeline (guest + checker) drove the runs end-to-end (every-guest-in-Cadenza, §9).
        harnessPrograms = cadenzaGuests;

        # `platformItest`: the `cdz-platform-itest` executable built ONCE (behind testing+host → wasmtime),
        # shared by every harness run so a test/program change never rebuilds it. Its src is the seed
        # workspace MINUS the guest sources and the harness-runs dir (editing a guest or a run must NOT bust
        # the binary's input hash) — the fine-grained-cache boundary the operator called for. Builds wasmtime
        # under stdenvNoCC+rustToolchain exactly as the crate-cdz check does; installs just the one binary.
        platformItestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.difference
            (pkgs.lib.fileset.unions [
              ./implementation/seed/crates
              ./implementation/compiler-ml
              ./xtask
              ./Cargo.toml
              ./Cargo.lock
              ./.cargo
              ./rust-toolchain.toml
              ./spec/semantics
            ])
            (pkgs.lib.fileset.unions [
              ./implementation/seed/crates/cdz-platform/guests
              ./implementation/seed/crates/cdz-platform/harness-runs
            ]);
        };
        platformItest = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-platform-itest";
          version = "0.0.0";
          src = platformItestSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
            cargo build --release --locked -p cdz-platform --bin cdz-platform-itest --features "testing host"
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            install -Dm755 target/release/cdz-platform-itest "$out/bin/cdz-platform-itest"
            runHook postInstall
          '';
        };

        # ── contract name→hash mapping (design/cadenza-platform.md section 1) ──────────────────────
        #
        # The operator design (cdz-contract/src/main.rs): a contract's identity is its content hash, and the
        # NAME→hash mapping is produced OUTSIDE the platform — by nix invoking `cdz-contract hash` over a
        # directory of contract sources — then fed to a run as data. The platform itself resolves only by
        # hash; a run may REFERENCE a contract by its stable name, which the harness transform (mkHarnessAst)
        # resolves to the hash via this mapping, exactly as it resolves a `program` name to a store path.
        #
        # `contractHasher`: the `cdz-contract` binary, built ONCE like platformItest (plain cargo over the
        # offline vendor, `-p cdz-contract` scoping the seed workspace). Reuses platformItestSrc so it shares
        # that source snapshot's cache and, like it, excludes the guests/harness-runs (the tool needs neither).
        contractHasher = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-contract";
          version = "0.0.0";
          src = platformItestSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
            cargo build --release --locked -p cdz-contract --bin cdz-contract
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            install -Dm755 target/release/cdz-contract "$out/bin/cdz-contract"
            runHook postInstall
          '';
        };

        # `cdzComponentRewrite`: the `cdz-component-rewrite` CLI, built ONCE like `contractHasher` (plain
        # cargo over the offline root vendor, `-p cdz-component-rewrite`). The value-heap runtime build
        # (`mkRuntime` via `stampNfc`) shells out to it to stamp the NFC component's content address INLINE
        # into the heap's `cadenza:nfc/normalize` import — making the heap self-describing (the runtime
        # resolves NFC purely from the import name, no `runtime.toml`/mapping; operator directive 2026-08-23),
        # exactly as `cargo xtask build` does. Reuses `platformItestSrc` (includes `crates/`) + `seedCargoVendor`.
        cdzComponentRewrite = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-component-rewrite";
          version = "0.0.0";
          src = platformItestSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
            cargo build --release --locked -p cdz-component-rewrite --bin cdz-component-rewrite
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            install -Dm755 target/release/cdz-component-rewrite "$out/bin/cdz-component-rewrite"
            runHook postInstall
          '';
        };

        # The corpus per-case pipeline's PHASE BINARIES (design/DESIGN-corpus-nix-per-case-caching.md),
        # each built ONCE like `contractHasher` with a MINIMAL closure so its derivation rotates only when
        # THAT phase's code changes — the caching discipline the design turns on:
        #   - `cdzCorpus` = the `cdz-corpus` bin (shred): parser closure. `cdz corpus records --out-dir`
        #     lives here, NOT in `seedCompiler` (built `--no-default-features`, so the corpus subcommand is
        #     absent there — and deliberately so, keeping corpus edits from rotating the compiler).
        #   - `cdzCompile` = the `cdz-compile` bin (build): compiler-only closure (rcdzc). A `--no-default`
        #     `cdz` has no standalone compile bin; this is the small one added for the pipeline's build phase.
        # (exec = `cdz-run`, already emitted by `seedCompiler`; it carries no compiler, so a compiler change
        # cannot invalidate the exec layer beyond the artifact input — the build/exec decoupling.)
        # `mkPhaseBin { pname; crate; bin; closure }` — build one phase bin from a source snapshot SCOPED to
        # its crate's dep-closure (`crateCompileSrc` per closure member + non-closure `Cargo.toml`s +
        # synthetic `stubNonClosure` stubs so cargo parses the workspace without the omitted src), exactly
        # like `seedCompilerSrc`/`seedCompiler`. Closure-scoping is what makes the caching REAL: because
        # `rcdzc` (the compiler) is NOT in `cdz-run`'s or `cdz-corpus`'s dep-closure, a compiler-source edit
        # is not in their snapshot → those bins CACHE-HIT → so does an exec keyed on them. A shared
        # whole-workspace snapshot (the old `platformItestSrc`) would rotate every bin on any edit and defeat
        # the exec/build decoupling (the emitted-wasm-unchanged ⇒ exec-cache-hit win).
        mkPhaseBin = { pname, crate, bin ? pname, closure, injectRuntimeHash ? false }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname;
            version = "0.0.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions (
                (pkgs.lib.concatMap crateCompileSrc closure)
                ++ nonClosureManifests closure
                ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]);
            };
            nativeBuildInputs = [ rustToolchain ];
            buildPhase = ''
              runHook preBuild
              chmod -R u+w .
              ${stubNonClosure closure}
              [ -f xtask/src/main.rs ] || { mkdir -p xtask/src; echo "fn main(){}" > xtask/src/main.rs; }
              [ -f xtask/src/lib.rs ] || echo "" > xtask/src/lib.rs
              ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
              ${pkgs.lib.optionalString injectRuntimeHash ''
                # Same nix-built-hash injection as `seedCompiler`: this compiler stamps the runtime/nfc content
                # hash of the components in THIS closure into the wasm it emits, so a program built here imports
                # the exact runtime the corpus EXEC resolves from `componentStore` — not the committed default,
                # which is host-specific (the runtime wasm is not byte-reproducible cross-host). Without it an
                # off-fleet corpus exec fails "no runtime of content address <committed-default> in the store".
                # rcdzc reads these via `option_env!` in `runtime_abi.rs`; the `cat` reuses the shared `hashOf`
                # derivations (no re-hash, no IFD). Only the COMPILER phase-bin sets this (the parser/exec bins
                # have no rcdzc in their closure, so it would be a no-op that needlessly rotates their cache).
                export CDZ_RUNTIME_HASH="$(cat ${runtimeHash})"
                export CDZ_DEBUG_RUNTIME_HASH="$(cat ${runtimeDebugHash})"
                export CDZ_NFC_HASH="$(cat ${nfcHash})"
              ''}
              cargo build --release --locked -p ${crate} --bin ${bin}
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              install -Dm755 target/release/${bin} "$out/bin/${bin}"
              runHook postInstall
            '';
          };
        # shred (parser closure — excludes rcdzc), build (compiler closure = rcdzc), exec (runtime closure —
        # cdz-run deps wasmtime/cadenza-syntax/cdz-contract/cdz-rt, NO rcdzc, so COMPILER-FREE by construction).
        cdzCorpus = mkPhaseBin { pname = "cdz-corpus"; crate = "cdz-corpus"; closure = crateClosure "cdz-corpus"; };
        cdzCompile = mkPhaseBin { pname = "cdz-compile"; crate = "rcdzc"; bin = "cdz-compile"; closure = crateClosure "rcdzc"; injectRuntimeHash = true; };
        cdzRun = mkPhaseBin { pname = "cdz-run"; crate = "cdz-run"; closure = crateClosure "cdz-run"; };
        # The RUST exec grader (`cdz-rust-run --grade`) — the rust-target analogue of `cdz-run` for the corpus
        # rust exec layer. COMPILER-FREE by construction (its closure is cdz-rust-run's deps: cdz-rust-render +
        # cdz-corpus-grade + cadenza-syntax, NO rcdzc), so a compiler change cannot rotate a rust exec beyond
        # the (content-addressed) build input. It shells the ambient `rustc` (provided by the exec derivation)
        # to compile the emitted driver, linking the pre-built `rustRlibs`.
        cdzRustRun = mkPhaseBin { pname = "cdz-rust-run"; crate = "cdz-rust-run"; closure = crateClosure "cdz-rust-run"; };
        # The wasm-opt optimality-gap FORMATTER (v-wasm-opt's `cdz-wasm-opt-gap`, #4537): parses a case's
        # `wasm-opt --metrics` output (ours vs -O3) + the three module sizes into ONE `(gap …)`/`(optimal …)`
        # sexpr record. Std-only + zero-dep so its closure is just itself — a tiny compiler-free phase bin. It
        # runs NO wasm-opt of its own (the per-case derivation does), so it stays trivially cacheable.
        cdzWasmOptGap = mkPhaseBin { pname = "cdz-wasm-opt-gap"; crate = "cdz-wasm-opt-gap"; bin = "wasm-opt-gap"; closure = crateClosure "cdz-wasm-opt-gap"; };

        # Just the contract sources — the narrowest input so the mapping re-derives only when a contract's
        # schema/pragmas change, not on any seed-crate edit. `cdz-contract hash` walks these `*.cdz`, parses
        # each with the pinned `cdz` binary, and emits the deterministic (sorted) name→base62-id JSON.
        contractSourcesDir = ./implementation/seed/crates/cdz-platform/contracts;
        contractHashes = pkgs.runCommand "cdz-contract-hashes"
          { nativeBuildInputs = [ contractHasher seedCompiler ]; } ''
          # `cdz-contract hash` now COMPILES+EXECUTES each contract's `descriptor()` to derive the id (the
          # operator's pragma-deprecation: identity flows through the guest's own self-reflection, no
          # `@!contract`/`@!input`/`@!output`). So this derivation gains two inputs beyond the old parse:
          #   --lib : the `contract-id` library the platform contracts `import … from "contract-id"` (it
          #           lives under guests/, not the hashed contracts/ dir) — passed through to `cdz compile`.
          #   CDZ_STORE : the value-heap runtime STORE, so the in-memory `cdz run --format binary-ast` step
          #           (which executes descriptor()) resolves the runtime the compiled contract imports by
          #           content address. `cdz run` inherits it from the ambient env (no --store flag). Same
          #           store mkCorpusExec runs against; descriptor() needs ONLY the value-heap runtime (no
          #           host imports/caps/world), so the store alone suffices.
          #   CDZ_COMPILE_BIN : the `--no-default-features` seedCompiler DELEGATES `cdz compile` to the
          #           external `cdz-compile` CLI (v-cdz-delegate #3397), so the `cdz compile` step needs it
          #           reachable — point at the content-addressed `cdzCompile`, exactly as seedCompiler's own
          #           env does (flake.nix ~L665/728). Without it: `cdz: cdz-compile not found`.
          export HOME="$TMPDIR/home"; mkdir -p "$HOME"
          export CDZ_STORE="${componentStore}"
          export CDZ_COMPILE_BIN="${cdzCompile}/bin/cdz-compile"
          # Stage the lib under its CLEAN name: `cdz compile` derives a package-file's module name from the
          # input's FILE STEM, and the import is `from "contract-id"`, so the input must be named
          # `contract-id.cdz`. The raw store path is `<hash>-contract-id.cdz` (stem `<hash>-contract-id`) →
          # CDZ0201 "unknown package file `contract-id`". Copy to a clean-named temp file and pass that.
          cp ${./implementation/seed/crates/cdz-platform/guests/contract-id.cdz} "$TMPDIR/contract-id.cdz"
          cdz-contract hash ${contractSourcesDir} \
            --lib "$TMPDIR/contract-id.cdz" \
            --cdz ${seedCompiler}/bin/cdz --out "$out"
        '';

        # The program names a run references — every `program = "<name>"` field in the ML spec. This is
        # DEPENDENCY DISCOVERY for the nix graph (which programs a run's derivation depends on, so caching is
        # per-program); the actual name→path substitution is an AST-validated `cdz rewrite` at build time
        # (see mkHarnessRun), never a text edit. Nix can't run `cdz query` at eval time, so the dependency
        # set is read from the spec text here; the transform stays structural.
        harnessProgramsIn = specText:
          let
            parts = builtins.split ''program[[:space:]]*=[[:space:]]*"([^"]+)"'' specText;
            caps = builtins.filter builtins.isList parts;
          in
          pkgs.lib.unique (map (c: builtins.elemAt c 0) caps);

        # The contract names a run references BY NAME — every `contract = "<name>"` STRING field in the ML
        # spec. The regex requires a `"` immediately after `=`, so it matches only the string form and never
        # `contract = b"…"` (a raw-bytes literal a spec may still use); a run can mix both. Same discovery role
        # as harnessProgramsIn: it tells the nix graph which contracts a run depends on so mkHarnessAst can
        # resolve each name to its hash from `contractHashes`, while the actual substitution stays a structural
        # `cdz rewrite` at build time. Reading from the spec text (not `cdz query`) avoids import-from-derivation.
        harnessContractsIn = specText:
          let
            parts = builtins.split ''contract[[:space:]]*=[[:space:]]*"([^"]+)"'' specText;
            caps = builtins.filter builtins.isList parts;
          in
          pkgs.lib.unique (map (c: builtins.elemAt c 0) caps);

        # Run ONE harness spec (an ML file). For each program the run references, `cdz rewrite` replaces its
        # `(= program "<name>")` node with `(= path "<nix wasm store path>")` — an AST-validated structural
        # rewrite, not a text edit — resolving the name to the reproducibly-built component. The transformed
        # ML is encoded to the Cadenza binary AST (`cdz convert --to binary`) and run by the shared
        # `platformItest` binary; the run passes iff the binary EXITS 0. Assertions about the observation log
        # belong to the harness/its checker (in the spec), NOT to nix. The derivation's inputs are exactly
        # {the shared binary, the programs this run uses, this spec}, so it re-runs only when one changes.
        # The TRANSFORM step, decoupled from the run (operator #2994: "the transforms and the runs need to be
        # decoupled, so the run doesn't rerun if the spec hash hasn't changed"). This derivation resolves the
        # spec's `program` (and, later, contract) names to nix paths/hashes via `cdz rewrite` and encodes the
        # result to the Cadenza binary AST. Its inputs are {the spec, the programs it uses, `cdz`} — NOT the
        # integration-test binary. So a binary change (frequent, from cdz-platform churn) reuses this cached
        # AST, and only the run step re-executes.
        mkHarnessAst = { name, specFile }:
          let
            specText = builtins.readFile specFile;
            uses = harnessProgramsIn specText;
            contractUses = harnessContractsIn specText;
            # Resolve each BLOB entry's `program = "<name>"` to `path = "<store-path>"` so the itest loads the
            # component bytes. SCOPED to a blob record `{ name = …, program = … }` (matched via the sibling
            # `name` field), NOT a bare `(= program …)`: a `pure-run = { program = "<name>", … }` field
            # references a seeded blob BY NAME and must stay `program` (the itest's pure-run decoder requires
            # it) — a bare rewrite also flipped THAT to `path`, so pure-run-emit-then-close failed spec-decode
            # "missing required field program". The blob record has a `name` sibling; pure-run does not, so the
            # record-scoped pattern hits only the blobs.
            rewrites = pkgs.lib.concatMapStringsSep "\n"
              (n: ''
                ${seedCompiler}/bin/cdz rewrite '("record" (= name ,nm) (= program "${n}"))' '("record" (= name ,nm) (= path "${harnessPrograms.${n}}"))' \
                  run.ml --from ml --to ml > run.ml.next
                mv run.ml.next run.ml
              '')
              uses;
            # Resolve each `contract = "<name>"` to its content hash from the name→id mapping, then rewrite the
            # node to `(= contract "<base62-id>")` (a delivery's contract is the base62 contract-id string, §8/§9).
            # The id is looked up at BUILD time (jq over the mapping file) rather than at eval — same no-IFD
            # discipline as the program rewrite. An unknown name is a hard error (never a silent skip); jq
            # yields the string "null" for a missing key, which we reject explicitly.
            contractRewrites = pkgs.lib.concatMapStringsSep "\n"
              (cname: ''
                id="$(${pkgs.jq}/bin/jq -r --arg n "${cname}" '.[$n]' ${contractHashes})"
                if [ "$id" = "null" ] || [ -z "$id" ]; then
                  echo "harness-ast '${name}': contract name '${cname}' is not in the contract-hash mapping" >&2
                  echo "(the mapping is ${contractHashes}; check the name matches an @!contract source)" >&2
                  exit 1
                fi
                ${seedCompiler}/bin/cdz rewrite "(= contract \"${cname}\")" "(= contract \"$id\")" \
                  run.ml --from ml --to ml > run.ml.next
                mv run.ml.next run.ml
              '')
              contractUses;
            # INJECT the run's self-contained deps (operator directive 2026-08-24): the harness run CARRIES
            # the runtime + nfc it needs — resolved by CONTENT HASH — instead of the host pulling them from an
            # env-pointed store, mirroring the self-describing nfc-stamp. Every `componentStore/*.wasm` (the
            # value-heap runtime + debug-counters runtime + nfc + guests) is added to a new top-level `deps`
            # field as an UNNAMED `{ path = … }` record; the itest (#3184) seeds `spec.deps` into EACH run's
            # CAS by hash, so a Cadenza guest's `cadenza:runtime/heap@…+<hash>` import composes and the guest
            # actually FOLDS (without it a guest silently fails to instantiate — no fold, a vacuous "exit 0").
            # The pattern targets the TOP-LEVEL record uniquely via its `registry` field (every run's required
            # event-registry field; nested blob/spawn records have none), so `deps` is added once, not to every
            # subrecord.
            depsInject = ''
              deps=""
              for f in ${componentStore}/*.wasm; do
                deps="$deps (\"record\" (= path \"$f\"))"
              done
              ${seedCompiler}/bin/cdz rewrite '("record" (= registry ,reg) ,@rest)' \
                "(\"record\" (= registry ,reg) (= deps (\"list\" $deps)) ,@rest)" \
                run.ml --from ml --to ml > run.ml.next
              mv run.ml.next run.ml
            '';
          in
          pkgs.runCommand "harness-ast-${name}" { } ''
            set -euo pipefail
            cp ${specFile} run.ml
            ${rewrites}
            ${contractRewrites}
            ${depsInject}
            ${seedCompiler}/bin/cdz convert --from ml --to binary run.ml > "$out"
          '';

        # The RUN step: execute the (already-transformed) binary AST with the shared `platformItest` binary;
        # pass iff it exits 0. Its inputs are {the transformed AST, the binary} — NOT the spec/programs — so
        # it re-runs only when the transform output OR the binary changes, and is a cache hit when the spec
        # (and thus its AST) is unchanged. Assertions about the log belong to the harness/its checker, not nix.
        mkHarnessRun = { name, specFile }:
          let ast = mkHarnessAst { inherit name specFile; };
          in
          pkgs.runCommand "harness-run-${name}" { } ''
            set -euo pipefail
            ${platformItest}/bin/cdz-platform-itest ${ast}
            echo "ok: harness run '${name}' exited 0 (ast ${ast})" > "$out"
          '';

        # The content address of a built component = the platform content address of its (stripped) bytes:
        # `Hash::of(HashTag::Blob, bytes)` rendered base62 (§8), computed by the `cdz-contract blob` CLI so it
        # is byte-identical to `cdz-run`'s / `xtask`'s `content_address` and the store's own `put()`. DERIVED
        # from the artifact nix built — the Cadenza content-address a program pins, falling out of the build
        # rather than being asserted. Exposed as a `packages.*-hash` (a plain-text store file).
        #   base62, NOT hex: the address must equal the `+<hash>` a program pins on its runtime import
        #   (`cadenza:runtime/heap@0.0.0+<hash>`), and that suffix rides a component-import semver
        #   build-metadata field whose grammar rejects base64url's `_` — so base62 (`0-9A-Za-z`) is the one
        #   text form everywhere (§8). Every content-address here — *-hash packages, componentStore filenames,
        #   runtime.toml, the parity checks vs REQUIRED_RUNTIME_HASH — routes through this same CLI, so a
        #   `hashOf` output can be handed straight to a `+suffix` or a `<hash>.wasm` filename.
        hashOf = drv: name:
          pkgs.runCommand name { nativeBuildInputs = [ contractHasher ]; } ''
            cdz-contract blob ${drv} > $out
          '';

        # The content address of each shipped component, derived ONCE (a single `hashOf` derivation apiece)
        # and shared by every consumer — the `packages.*-hash` outputs, the `componentStore` filenames +
        # runtime.toml, the compiler-hash injection (`seedCompiler`), the NFC-stamp (`mkRuntime`). Each is a
        # tiny store file holding the base62 hash; a consumer reads it with `cat ${…Hash}` at build time (no
        # IFD) instead of re-running `cdz-contract blob` in its own buildPhase. So the hash is computed once
        # per component (cached, shared) rather than once per use-site — one source of truth for "the hash of
        # this blob."
        runtimeHash = hashOf runtime "cdz-runtime-hash";
        runtimeDebugHash = hashOf runtimeDebug "cdz-runtime-debug-hash";
        nfcHash = hashOf nfc "cdz-nfc-hash";

        # ── R2: the content-addressed component STORE ─────────────────────────────────────────────
        #
        # Assemble every nix-built component into ONE store directory, each file named by its DERIVED
        # content hash: `<blake3>.wasm`. This mirrors `target/cadenza-store` (what `xtask build`
        # produces) but built + addressed BY NIX — the store the operator's north star describes, from
        # which a cadenza runtime / the harness loads a component by hash. Purely a function of the
        # component derivations, so it's cache-shareable + rebuilt only when a component changes.
        # (A later increment has the runtime/harness RESOLVE from this store; that's a cross-territory
        # change coordinated with v-runtime + the harness — this increment only PRODUCES the store.)
        # Filenames + runtime.toml use the platform content address (`cdz-contract blob` = base62 Blob hash),
        # matching `content_address` / the store's `put()` so a program resolves the runtime identically here.
        componentStore = pkgs.runCommand "cdz-component-store"
          {
            # CONTENT-ADDRESSED (operator 2026-08-25 "we should be using CAS for sure"): the store's output
            # PATH is a function of its CONTENTS (the runtime/nfc wasm keyed by hash + runtime.toml), not of
            # its build inputs. So when a change rebuilds the store's build-tools (`cdz-contract` etc. share
            # the wide workspace snapshot, so a compiler-source edit rebuilds them) but the wasm bytes are
            # unchanged, the store re-derives to the SAME path — and every consumer keyed on it (notably the
            # corpus `exec` derivations, which resolve the runtime from `CDZ_STORE`) CACHE-HITS. This is the
            # last compiler-taint on the build/exec decoupling: without it a compiler change with identical
            # emit still rotated the store path and re-ran every exec.
            __contentAddressed = true;
            outputHashMode = "recursive";
            outputHashAlgo = "sha256";
          } ''
          set -euo pipefail
          mkdir -p "$out"
          # Store every component (both heaps + the NFC dependency) by its content address — a pure CAS.
          # The heaps import `cadenza:nfc/normalize@0.0.0+<nfc-hash>` inline (stamped by `mkRuntime`), so a
          # runtime resolves its NFC dependency to `<store>/<nfc-hash>.wasm` DIRECTLY from the import — the
          # NFC file must be present here, but no manifest maps to it. Filenames use the SHARED `hashOf`
          # derivations (`cat` here, not re-hashed), so the store keys agree by construction with the
          # compiler-injected + guest-imported + parity-checked hashes — one source of truth per blob.
          rt=$(cat ${runtimeHash})
          dbg=$(cat ${runtimeDebugHash})
          nfch=$(cat ${nfcHash})
          ${pkgs.coreutils}/bin/cp ${runtime} "$out/$rt.wasm"
          ${pkgs.coreutils}/bin/cp ${runtimeDebug} "$out/$dbg.wasm"
          ${pkgs.coreutils}/bin/cp ${nfc} "$out/$nfch.wasm"
          # An INFORMATIONAL manifest of the heap builds — NOT read by any executable to resolve anything
          # (the NFC dep is self-describing inline in each heap; operator directive 2026-08-23: no mapping
          # file passed to executables). Mirrors `xtask build`'s runtime.toml (no `nfc =` line).
          cat > "$out/runtime.toml" <<EOF
          # Cadenza content-addressed store — the value-heap runtime builds (informational; the NFC
          # dependency is resolved from each heap's self-describing inline import, not from this file).
          runtime = "$rt"
          debug_runtime = "$dbg"
          EOF
        '';

        # ── The corpus per-case caching graph (design/DESIGN-corpus-nix-per-case-caching.md) ────────────
        #
        # Each corpus case flows through THREE separately-cached derivations — shred → build → exec — so an
        # unrelated change is a cache HIT, and (the headline) a COMPILER change that leaves a case's emitted
        # wasm byte-identical does NOT rerun that case's exec. The build derivation is CONTENT-ADDRESSED
        # (`__contentAddressed`): its output path is a function of the emitted BYTES, not its inputs — so a
        # compiler change reruns the build (its input `cdzCompile` rotated) but, when the wasm is identical,
        # yields the SAME output path, and the exec keyed on it cache-hits. Input-addressed derivations
        # could not express this (the build's path would change with the compiler even for identical output).
        #
        # Cases are enumerated at EVAL time from the SOURCE `.sexp` (a pure regex count of `(case "…"` forms,
        # like `harnessProgramsIn`) — the flake avoids IFD entirely, so nix never reads a built manifest.
        # The shred still emits per-case dirs at build time; the eval-time count gives the `0000..N-1` indices
        # whose dirs it names, and each per-case derivation globs its own `<idx>-*` dir out of the shred.
        corpusCaseCount = file:
          let
            # LINE-ANCHORED count: a top-level `(case "…"` starts at column 0, so match a `(case "` preceded
            # by a NEWLINE. A naive `\(case "` (match anywhere) over-counts — a `(case "` embedded in a
            # COMMENT or in program DATA (e.g. 12-metaprogramming's quasiquote examples: `; … (case "…"`)
            # is not a real top-level case but the parser (shred) doesn't emit a dir for it, so the extra
            # index had "no shred dir" and the per-case build failed. Prepend a newline so a case on the
            # very first line is still anchored. `[[:space:]]` here matches only the spaces AFTER `case` (the
            # anchor is the literal newline), so an inline `… (case "` in a comment is excluded. Nix POSIX
            # regex rejects `\n`, so the pattern carries a LITERAL newline (double-quoted `\n`).
            txt = "\n" + builtins.readFile file;
            caps = builtins.filter builtins.isList (builtins.split "\n\\(case[[:space:]]+\"" txt);
          in
          builtins.length caps;
        # ALL case TITLES of a file, extracted at EVAL time with ONE split over the SOURCE `.sexp` (same
        # line-anchored `(case "…"` split as corpusCaseCount, no IFD). Computed ONCE PER FILE — the earlier
        # per-idx `corpusCaseTitle file idx` re-read+re-split the whole file for EVERY case, i.e. O(N²) per
        # file (1375 full-file splits for 05-compound-types), which made the whole-corpus wasm-opt-gaps eval
        # pathologically slow / fail. The split yields `[pre, seg0, seg1, …]` where `seg<i>` is the text right
        # after the i-th `(case "`; `tail` drops the pre-string, so the result is EXACTLY one title per case
        # (length == corpusCaseCount), safely indexable 0..N-1. Each title = its segment up to the first `"`
        # (corpus titles carry no embedded quote, verified). Used to LABEL an opt-gap record WITHOUT a
        # whole-file-shred dependency, so a single case's edit re-runs ONLY its own (CA-keyed) opt-gap.
        corpusCaseTitles = file:
          let
            strs = builtins.filter builtins.isString
              (builtins.split "\n\\(case[[:space:]]+\"" ("\n" + builtins.readFile file));
          in
          map (seg: builtins.head (builtins.split "\"" seg)) (builtins.tail strs);

        # SHRED (content-addressed) — parse a whole corpus file into per-case artifact dirs, ONCE. Closure =
        # the parser (`cdzCorpus`); reruns only when the `.sexp` changes. Clean-name copy first: `cdz-corpus`
        # names the subdir by the input stem, and a store path is hash-prefixed (`<hash>-01-literals`), so the
        # copy makes the subdir exactly `${name}/` (the `mkCadenzaGuest` clean-name idiom).
        mkCorpusShred = { name, file }:
          pkgs.runCommand "corpus-shred-${name}"
            {
              nativeBuildInputs = [ cdzCorpus ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            cp ${file} ${name}.sexp
            cdz-corpus records --out-dir "$out" ${name}.sexp
          '';

        # BUILD (content-addressed) — compile ONE case's native artifacts to wasm, capturing the outcome.
        # Closure = the compiler (`cdzCompile`). Output carries everything the exec needs so exec keys ONLY
        # on this (content-addressed) output + `cdzRun`: `emit.wasm` (on success), `compile.status`,
        # `compile.err`, and the run metadata forwarded from the shred (`test-run.ast`, `expect-kind`,
        # `component-name`). Because it is content-addressed, a compiler change that re-emits identical
        # bytes + identical metadata produces the SAME output path → the exec cache-hits.
        mkCorpusBuild = { name, shred, idx }:
          pkgs.runCommand "corpus-build-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            case=$(echo ${shred}/${name}/${idx}-*)
            [ -d "$case" ] || { echo "no shred dir for case ${idx} of ${name}"; exit 1; }

            inputs=("ast:main=$case/program.ast")
            entry=()
            for m in "$case"/module-*.ast; do
              if [ -e "$m" ]; then
                n=$(basename "$m" .ast); n=''${n#module-}
                inputs+=("ast:$n=$m")
                entry=(--entry main)
              fi
            done
            cfg=()
            if [ -e "$case/wit-world.ast" ]; then cfg+=("wit-world:w=$case/wit-world.ast"); fi
            if [ -e "$case/component-name" ]; then cfg+=(--component-name "$(cat "$case/component-name")"); fi

            # Compile. A refusal (error/declines case) is NOT a derivation failure — capture the outcome; the
            # exec grades it. `emit.wasm` is present only on success.
            if cdz-compile "''${inputs[@]}" "''${cfg[@]}" "''${entry[@]}" -t wasm -o "$out/emit.wasm" 2>"$out/compile.err"; then
              printf '0' > "$out/compile.status"
            else
              printf '%s' "$?" > "$out/compile.status"
            fi
            # (peer) CROSS-COMPONENT cases (L3): a case may ship provider PEERS the consumer imports via
            # `(extern …)`. Each `peer-N.ast` is a STANDALONE provider program compiled EXACTLY like
            # `program.ast` but with `--component-name <iface>` (from the `peer-N.iface` sidecar) — that is
            # how a world-less provider exports its interface. Compile each into this (content-addressed)
            # build output so the exec composes them via `cdz-run --peer`; a peer compile failure appends to
            # `compile.err` (the exec grades the consumer's outcome). The iface has `:`/`/` (not
            # filename-safe) so it rides the `.iface` sidecar, not the stem.
            for p in "$case"/peer-*.ast; do
              [ -e "$p" ] || continue
              pn=$(basename "$p" .ast)                 # peer-N
              cdz-compile "ast:main=$p" --component-name "$(cat "$case/$pn.iface")" -t wasm \
                -o "$out/$pn.wasm" 2>>"$out/compile.err" || true
              cp "$case/$pn.iface" "$out/$pn.iface"
            done
            # Forward the run metadata so exec depends ONLY on this build output (+ cdzRun) — compiler-free.
            cp "$case/test-run.ast" "$out/test-run.ast"
            cp "$case/expect-kind" "$out/expect-kind"
            if [ -e "$case/component-name" ]; then cp "$case/component-name" "$out/component-name"; fi
          '';

        # EXEC — grade one case. Closure = the COMPILER-FREE `cdzRun` + the runtime store; NO compiler, so a
        # compiler change cannot rotate this beyond the (content-addressed) build input. `cdz-run --grade` is
        # the UNIVERSAL grader (reproduces the `xtask gate` for every outcome kind): it runs the wasm for
        # output/trap, and grades error/declines + warns from the captured compile outcome
        # (`--compile-status`/`--compile-diag`). So exec just hands it the build output — no bash routing.
        # `emit.wasm` is passed only when the compile succeeded (a refusal has none). Exit 1 = a real Fail.
        mkCorpusExec = { name, build, idx }:
          pkgs.runCommand "corpus-exec-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --baseline ${./spec/semantics/.gate-baseline})
            if [ -e ${build}/emit.wasm ]; then args=(${build}/emit.wasm "''${args[@]}"); fi
            if [ -e ${build}/component-name ]; then args+=(--component-name "$(cat ${build}/component-name)"); fi
            # (peer) L3: compose each provider peer the build compiled — `--peer <iface>=<peer-wasm>` binds
            # the peer's exported interface into the consumer's like-named `(extern …)` (run_with_peers).
            for pw in ${build}/peer-*.wasm; do
              [ -e "$pw" ] || continue
              pn=$(basename "$pw" .wasm)               # peer-N
              args+=(--peer "$(cat ${build}/$pn.iface)=$pw")
            done
            # HEAP-BALANCE (opt-out heap-liveness): under the opt-out default EVERY heap-importing case must
            # end at its expected live-cell count (0 by default, or the case's explicit / known-leak N), so
            # every wasm exec runs on the DEBUG-COUNTERS runtime — the shipped one reports 0 vacuously, only
            # the debug build has the real live-cell export `cdz-run --grade` reads. OVERRIDE the runtime with
            # `runtimeDebug` unconditionally (cdz-run composes it only when the component imports the heap; a
            # scalar/const case ignores the override, so this is safe — the grade skips the balance check for
            # a no-heap case). `runtimeDebug` is a store-path, so every exec's identity is a function of the
            # debug-runtime and reruns iff it changes (a minor accepted coarsening — formerly only annotated
            # cases carried this dep).
            args+=(--runtime ${runtimeDebug})
            cdz-run "''${args[@]}"
            echo "ok: corpus ${name} case ${idx}" > "$out"
          '';

        # ── corpus-cadenza: the CADENZA round-trip VALUE-equivalence target (v-cadenza-backend, #4759) ──────
        # `corpus` (wasm) WITH A CADENZA HOP: compile program.ast → cadenza (`program1.ast`, the OPTIMIZED
        # binary AST), then program1.ast → wasm (emit.wasm), and grade emit.wasm with the SAME `cdz-run`
        # against the SAME wasm `.gate-baseline`. Every case the cadenza backend EMITS must grade IDENTICALLY
        # to the direct-wasm path — a cadenza round-trip that changes a VALUE shows as a grade divergence,
        # catching value-miscompiles that byte-idempotence (a stable-but-wrong encode, e.g. a `-0.0` flag byte)
        # would miss. A cadenza DECLINE (the backend is early) is SKIPPED, not a regression (we only measure
        # emitted cases). Owner split: v-cadenza-backend owns the backend + round-trip semantics; v-nix owns
        # this flake mechanism (mirrors mkCorpusBuild/Exec, reuses the shred + wasm baseline + cdz-run grader).
        mkCorpusCadenzaBuild = { name, shred, idx }:
          pkgs.runCommand "corpus-cadenza-build-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            case=$(echo ${shred}/${name}/${idx}-*)
            [ -d "$case" ] || { echo "no shred dir for case ${idx} of ${name}"; exit 1; }

            inputs=("ast:main=$case/program.ast")
            entry=()
            for m in "$case"/module-*.ast; do
              if [ -e "$m" ]; then
                n=$(basename "$m" .ast); n=''${n#module-}
                inputs+=("ast:$n=$m")
                entry=(--entry main)
              fi
            done
            cfg=()
            if [ -e "$case/wit-world.ast" ]; then cfg+=("wit-world:w=$case/wit-world.ast"); fi
            if [ -e "$case/component-name" ]; then cfg+=(--component-name "$(cat "$case/component-name")"); fi

            # HOP 1: program.ast → CADENZA (program1.ast). A DECLINE (cadenza backend early) → SKIP marker,
            # no program1/emit — the exec skips (a decline must NOT read as a wasm-baseline regression).
            if cdz-compile "''${inputs[@]}" "''${entry[@]}" -t cadenza -o "$out/program1.ast" 2>"$out/compile.err"; then
              # HOP 2: the optimized program1.ast → WASM (emit.wasm), forwarding world/component-name. An
              # UN-compilable program1 (HOP2 fail) is a real cadenza bug → graded (reds), NOT skipped.
              if cdz-compile "ast:main=$out/program1.ast" "''${cfg[@]}" -t wasm -o "$out/emit.wasm" 2>>"$out/compile.err"; then
                printf '0' > "$out/compile.status"
              else
                printf '%s' "$?" > "$out/compile.status"
              fi
            else
              st=$?
              touch "$out/cadenza-declined"
              printf '%s' "$st" > "$out/compile.status"
            fi
            # (peer) cadenza-hop each provider peer the same way (cadenza → wasm) so the exec composes them via
            # `--peer` exactly like the wasm corpus; a peer whose cadenza hop declines is left absent.
            for p in "$case"/peer-*.ast; do
              [ -e "$p" ] || continue
              pn=$(basename "$p" .ast)
              if cdz-compile "ast:main=$p" -t cadenza -o "$out/$pn.ast1" 2>>"$out/compile.err"; then
                cdz-compile "ast:main=$out/$pn.ast1" --component-name "$(cat "$case/$pn.iface")" -t wasm \
                  -o "$out/$pn.wasm" 2>>"$out/compile.err" || true
              else
                # (v-cadenza-backend) SKIP the whole case if ANY component's cadenza hop declines — a
                # cross-component case is only meaningful when EVERY part round-trips; a partial hop would
                # leave the consumer's peer import unsatisfied and grade divergently (a false red, not a
                # value-miscompile). Mark declined so the exec skips (like a consumer-hop decline).
                touch "$out/cadenza-declined"
              fi
              cp "$case/$pn.iface" "$out/$pn.iface"
            done
            cp "$case/test-run.ast" "$out/test-run.ast"
            cp "$case/expect-kind" "$out/expect-kind"
            if [ -e "$case/component-name" ]; then cp "$case/component-name" "$out/component-name"; fi
          '';

        # EXEC — the wasm grader EXACTLY (cdz-run --grade vs the wasm baseline), plus ONE guard: a cadenza
        # DECLINE (build marked `cadenza-declined`) is SKIPPED (exit 0), not graded.
        mkCorpusCadenzaExec = { name, build, idx }:
          pkgs.runCommand "corpus-cadenza-exec-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            if [ -e ${build}/cadenza-declined ]; then
              echo "skip: corpus-cadenza ${name} case ${idx} — cadenza backend declined (early)" > "$out"
              exit 0
            fi
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --baseline ${./spec/semantics/.gate-baseline})
            if [ -e ${build}/emit.wasm ]; then args=(${build}/emit.wasm "''${args[@]}"); fi
            if [ -e ${build}/component-name ]; then args+=(--component-name "$(cat ${build}/component-name)"); fi
            for pw in ${build}/peer-*.wasm; do
              [ -e "$pw" ] || continue
              pn=$(basename "$pw" .wasm)
              args+=(--peer "$(cat ${build}/$pn.iface)=$pw")
            done
            args+=(--runtime ${runtimeDebug})
            cdz-run "''${args[@]}"
            echo "ok: corpus-cadenza ${name} case ${idx}" > "$out"
          '';

        corpusCadenzaCaseChecks = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in
          builtins.listToAttrs (map
            (idx: {
              name = "${name}-${idx}";
              value = mkCorpusCadenzaExec {
                inherit name idx;
                build = mkCorpusCadenzaBuild { inherit name shred idx; };
              };
            })
            idxs);

        mkCorpusCadenzaFileAgg = { name, file }:
          let cases = corpusCadenzaCaseChecks { inherit name file; };
          in
          assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "corpus-cadenza-${name}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: corpus-cadenza ${name} — ${toString (builtins.length (builtins.attrNames cases))} cases via per-case shred→cadenza-build→exec" > "$out"
          '';

        corpusCadenzaFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            {
              name = "corpus-cadenza-${stem}";
              value = mkCorpusCadenzaFileAgg { name = stem; file = ./spec/semantics + "/${f}"; };
            })
          corpusFileNames);
        corpusCadenzaAll = pkgs.runCommand "corpus-cadenza-all" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues corpusCadenzaFileAggs)}
          echo "ok: corpus-cadenza — ${toString (builtins.length corpusFileNames)} files graded via the per-case shred→cadenza-build→exec caching graph" > "$out"
        '';

        # A corpus file's per-case check MAP `{ "<idx>" = execDrv; … }` — shred once, then one build+exec
        # chain per case. `pipeline`-style (no barrier): each case is an independent chain.
        corpusCaseChecks = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in
          builtins.listToAttrs (map
            (idx: {
              name = "${name}-${idx}";
              value = mkCorpusExec {
                inherit name idx;
                build = mkCorpusBuild { inherit name shred idx; };
              };
            })
            idxs);

        # The per-FILE aggregate check: every case's exec must pass. `cat`-ing each exec marker adds the
        # store dependency (string context) without a buildInput, so `nix build .#checks.<sys>.corpus-<name>`
        # forces the whole graph; a red case fails here. Non-vacuity guarded by the eval-time count > 0.
        mkCorpusFileAgg = { name, file }:
          let cases = corpusCaseChecks { inherit name file; };
          in
          assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "corpus-${name}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: corpus ${name} — ${toString (builtins.length (builtins.attrNames cases))} cases via per-case shred→build→exec" > "$out"
          '';

        # Every compiler-genre corpus file (all of `spec/semantics/*.sexp` — the platform-genre corpus lives
        # under `spec/platform`, never here). Enumerated at EVAL time from the SOURCE dir (`readDir`, no IFD).
        corpusFileNames = builtins.filter (pkgs.lib.hasSuffix ".sexp")
          (builtins.attrNames (builtins.readDir ./spec/semantics));
        # `corpus-<file>` per-file aggregates, mapped over every corpus file — each shreds its file once and
        # runs a per-case build→exec chain. The `corpus` TOP-LEVEL aggregate forces them all (so `nix flake
        # check` covers the whole corpus through the per-case caching graph, and CI can build/cache one file
        # or one case in isolation).
        corpusFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            {
              name = "corpus-${stem}";
              value = mkCorpusFileAgg { name = stem; file = ./spec/semantics + "/${f}"; };
            })
          corpusFileNames);
        corpusAll = pkgs.runCommand "corpus-all" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues corpusFileAggs)}
          echo "ok: corpus — ${toString (builtins.length corpusFileNames)} files graded via the per-case shred→build→exec caching graph" > "$out"
        '';

        # ── wasm-opt OPTIMALITY-GAP sweep (operator 2026-08-27; design/DESIGN-wasm-opt-gap-analysis-rcdzc.md) ──
        # For every corpus wasm output that COMPILES, measure the gap between our emit and what Binaryen's
        # `wasm-opt` would produce. If wasm-opt shrinks nothing, our module is OPTIMAL on the metrics we track;
        # any reduction is a tracked, ADVISORY (never a gate-fail) emit-side backend TODO. rcdzc emits a
        # COMPONENT and Binaryen can't parse components (binaryen#6728), so per case we UNBUNDLE the core
        # module(s) first, then run `wasm-opt --all-features -O3`/`-Oz` + `--metrics` and hand the sizes+metrics
        # to `cdz-wasm-opt-gap` for one `(gap …)`/`(optimal …)` record. `--all-features` because our core uses
        # `return_call` (tail calls) — a bare wasm-opt fails the validator + would under-report a gap. Per-case
        # derivations depend ONLY on {emit.wasm (build), binaryen, wasm-tools, the formatter} — NEVER the
        # whole-file shred — so a single case's edit re-runs ONLY its own wasm-opt (keyed on its
        # content-addressed emit.wasm), not every case in the file. The case TITLE is passed at EVAL time
        # (corpusCaseTitle), not read from the shred, precisely to keep that per-case caching isolation. Nix
        # runs them IN PARALLEL exactly like the per-case test execs.
        mkCorpusOptGap = { name, build, idx, caseTitle }:
          pkgs.runCommand "wasm-opt-gap-${name}-${idx}"
            {
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
              nativeBuildInputs = [ pkgs.wasm-tools pkgs.binaryen cdzWasmOptGap ];
            } ''
            set -euo pipefail
            caseid=${pkgs.lib.escapeShellArg "${name} :: ${caseTitle}"}
            # A refused compile (an error/decline case) has no emit.wasm — nothing to optimize; write a skip
            # marker the aggregator drops (NOT an `optimal` claim — we simply did not measure it).
            if [ ! -e ${build}/emit.wasm ]; then
              echo "; skip (case \"$caseid\") no-emit" > "$out"; exit 0
            fi
            mkdir -p mods
            # Extract the embedded core module(s); `--threshold 0` grabs even tiny ones. An ordinary program is
            # one core module (`unbundled-module0.wasm`); a resource-escape/dtor program emits several — each
            # gets its own record (distinguished by `--module N`).
            wasm-tools component unbundle ${build}/emit.wasm --module-dir mods --threshold 0 -o /dev/null
            : > "$out"
            i=0
            for m in mods/*.wasm; do
              [ -e "$m" ] || continue
              orig=$(wc -c < "$m")
              wasm-opt --all-features -O3 "$m" -o o3.wasm
              wasm-opt --all-features -Oz "$m" -o oz.wasm
              o3=$(wc -c < o3.wasm); oz=$(wc -c < oz.wasm)
              # `--metrics` is a pass: bare = OUR module's metrics; appended after -O3 = the optimized metrics.
              wasm-opt --all-features     --metrics "$m" -o /dev/null > ours.metrics 2>/dev/null || true
              wasm-opt --all-features -O3 --metrics "$m" -o /dev/null > o3.metrics   2>/dev/null || true
              wasm-opt-gap --case "$caseid" --module "$i" --orig "$orig" --o3 "$o3" --oz "$oz" \
                --metrics-ours ours.metrics --metrics-opt o3.metrics >> "$out"
              printf '\n' >> "$out"
              i=$((i + 1))
            done
          '';

        # Per-FILE opt-gap report LIST (one derivation per case), mirroring corpusCaseChecks: shred once, build
        # per case, opt-gap per case. Independent → Nix parallelizes; each report is CA on {emit, binaryen}.
        corpusOptGapReports = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            titles = corpusCaseTitles file;   # ONE split for the whole file, indexed per case below
          in
          # DEFENSIVE: corpusCaseCount and corpusCaseTitles derive from the SAME line-anchored `(case "` split,
          # so their lengths agree by construction — but if a future/malformed file ever desynced them, the
          # per-case `elemAt titles i` would surface as a cryptic `elemAt index N on size N`. Fail loud + NAMED
          # instead (which file, both counts), so a bad case is a one-line diagnosis, not an OOB hunt.
          if builtins.length titles != n
          then throw "wasm-opt-gaps ${name}: corpusCaseTitles (${toString (builtins.length titles)}) != corpusCaseCount (${toString n}) — a malformed or embedded-quote (case …)?"
          else
            builtins.genList
              (i: mkCorpusOptGap {
                inherit name;
                idx = pkgs.lib.fixedWidthNumber 4 i;
                caseTitle = builtins.elemAt titles i;
                build = mkCorpusBuild { inherit name shred; idx = pkgs.lib.fixedWidthNumber 4 i; };
              })
              n;

        # AGGREGATOR: collect a set of per-case records, TALLY them by kind (for the self-describing summary),
        # DROP the optimal/skip markers, sort the `(gap …)` records by o3-delta DESC, wrap in the top-level
        # `(wasm-opt-gaps …)` form. Pure reduction (no wasm-opt) so it re-runs only when a per-case report
        # changed. `from-trunk` rides the flake rev — only in the aggregator (the per-case CA reports stay
        # rev-independent, so they cache across commits). The `(summary …)` header makes "near-optimal %" =
        # optimal/(optimal+gaps) computable from the report alone (v-wasm-opt request) — the counts are tallied
        # by each report's leading token (`(gap` / `(optimal` / `; skip`) BEFORE the gap-only records are kept.
        mkOptGapAgg = { drvName, reports }:
          pkgs.runCommand drvName { } ''
            set -euo pipefail
            idx=$(mktemp); kinds=$(mktemp)
            ${pkgs.lib.concatMapStringsSep "\n" (r: ''
              p=$(head -c4 ${r} 2>/dev/null || true)
              if [ "$p" = "(gap" ]; then
                echo gap >> "$kinds"
                d=$(grep -oE '\(delta \(o3 -?[0-9]+\)' ${r} | grep -oE -- '-?[0-9]+' | head -1)
                printf '%s\t%s\n' "''${d:-0}" "${r}" >> "$idx"
              elif [ "$p" = "(opt" ]; then echo optimal >> "$kinds"
              else echo skipped >> "$kinds"; fi
            '') reports}
            gaps=$(grep -cx gap "$kinds" || true);        gaps=''${gaps:-0}
            optimal=$(grep -cx optimal "$kinds" || true);  optimal=''${optimal:-0}
            skipped=$(grep -cx skipped "$kinds" || true);  skipped=''${skipped:-0}
            measured=$((optimal + gaps)); total=$((measured + skipped))
            {
              echo "(wasm-opt-gaps"
              echo "  (binaryen \"${pkgs.binaryen.version}\")"
              echo "  (from-trunk \"${self.shortRev or "dev"}\")"
              echo "  (summary (total-cases $total) (measured $measured) (optimal $optimal) (gaps $gaps) (skipped $skipped))"
              if [ -s "$idx" ]; then
                sort -k1,1 -rn "$idx" | cut -f2 | while IFS= read -r r; do sed 's/^/  /' "$r"; echo; done
              fi
              echo ")"
            } > "$out"
          '';
        mkOptGapFileAgg = { name, file }: mkOptGapAgg { drvName = "wasm-opt-gaps-${name}"; reports = corpusOptGapReports { inherit name file; }; };
        # `wasm-opt-gaps-<file>` per-file aggregates + one whole-corpus sweep (aggregates ALL per-case reports —
        # the SAME CA reports the per-file aggs use, so they share/cache).
        optGapFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            { name = "wasm-opt-gaps-${stem}"; value = mkOptGapFileAgg { name = stem; file = ./spec/semantics + "/${f}"; }; })
          corpusFileNames);
        optGapAll = mkOptGapAgg {
          drvName = "wasm-opt-gaps-all";
          reports = builtins.concatMap
            (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in corpusOptGapReports { name = stem; file = ./spec/semantics + "/${f}"; })
            corpusFileNames;
        };

        # ── The RUST exec layer (design gap #6) ──────────────────────────────────────────────────────────
        #
        # The rust-target twin of the wasm corpus graph above: the SAME per-case shred (reused verbatim —
        # native artifacts are backend-independent) → a rust BUILD (`cdz-compile -t rust` → emitted `.rs`) →
        # a rust EXEC (`cdz-rust-run --grade` compiles the emitted `.rs`'s driver with `rustc` + grades). The
        # rust backend declines many constructs today; a refused compile on an output/trap case grades Todo
        # (never Fail), exactly like the `xtask` rust gate — so a mostly-declining file is green. Because the
        # corpus `(output …)` value IS the wasm oracle's value (the corpus invariant), grading rust against
        # the corpus directly is equivalent to the xtask rust gate's differential grade vs the wasm oracle.

        # The pre-built runtime rlibs the rust exec links: `cdz_rt` (async `CdzEnv`), `cdz_num` (`cdz_num::Big`),
        # `cadenza_ast` (the native R2 value codec) + its transitive `num_bigint` in `deps/`. Mirrors xtask
        # `build_tools`' `cargo build -p cdz-rt -p cdz-num -p cadenza-ast`, copying the rlibs + `deps/` out. Built
        # ONCE and INPUT-addressed (not CA): a compiler (`rcdzc`) edit is NOT in these crates' closures, so the
        # rlibs stay cached across compiler changes; only a runtime-crate edit rebuilds them. `cdz-rust-run`
        # links each via `-L dependency=<dir> --extern <crate>=<dir>/lib<crate>.rlib` — all three point here.
        rustRlibs =
          let
            closure = pkgs.lib.unique (
              crateClosure "cdz-rt" ++ crateClosure "cdz-num" ++ crateClosure "cadenza-ast");
          in
          pkgs.stdenvNoCC.mkDerivation {
            pname = "cdz-rust-rlibs";
            version = "0.0.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions (
                (pkgs.lib.concatMap crateCompileSrc closure)
                ++ nonClosureManifests closure
                # cdz-num `#[path]`-includes the runtime's bigint.rs from a SIBLING crate (a source-share, not
                # a dep-graph edge, so crateClosure/crateCompileSrc miss it) — add it, as the crane per-crate
                # cdz-num/cdz-calc/rcdzc checks do (`extraSrc`).
                ++ [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]
                ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]);
            };
            nativeBuildInputs = [ rustToolchain ];
            buildPhase = ''
              runHook preBuild
              chmod -R u+w .
              ${stubNonClosure closure}
              [ -f xtask/src/main.rs ] || { mkdir -p xtask/src; echo "fn main(){}" > xtask/src/main.rs; }
              [ -f xtask/src/lib.rs ] || echo "" > xtask/src/lib.rs
              ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
              cargo build --release --locked -p cdz-rt -p cdz-num -p cadenza-ast
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p "$out/deps"
              cp target/release/libcdz_rt.rlib "$out/"
              cp target/release/libcdz_num.rlib "$out/"
              cp target/release/libcadenza_ast.rlib "$out/"
              cp -r target/release/deps/. "$out/deps/"
              runHook postInstall
            '';
          };

        # BUILD (content-addressed) — compile ONE case's native artifacts to RUST, capturing the outcome. The
        # rust twin of `mkCorpusBuild`: `-t rust` → `emit.rs` on success; a refusal captures `compile.status`/
        # `compile.err` (the exec grades it). Forwards `test-run.ast` so the exec keys ONLY on this output +
        # `cdzRustRun` + `rustRlibs`. (No `--component-name`/`wit-world`: the rust backend has no interface
        # export; a case that needs one declines here → Todo.)
        mkCorpusRustBuild = { name, shred, idx }:
          pkgs.runCommand "corpus-rust-build-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            case=$(echo ${shred}/${name}/${idx}-*)
            [ -d "$case" ] || { echo "no shred dir for case ${idx} of ${name}"; exit 1; }

            inputs=("ast:main=$case/program.ast")
            entry=()
            for m in "$case"/module-*.ast; do
              if [ -e "$m" ]; then
                n=$(basename "$m" .ast); n=''${n#module-}
                inputs+=("ast:$n=$m")
                entry=(--entry main)
              fi
            done

            if cdz-compile "''${inputs[@]}" "''${entry[@]}" -t rust -o "$out/emit.rs" 2>"$out/compile.err"; then
              printf '0' > "$out/compile.status"
            else
              printf '%s' "$?" > "$out/compile.status"
            fi
            cp "$case/test-run.ast" "$out/test-run.ast"
          '';

        # EXEC — grade one case's RUST emit. Closure = the COMPILER-FREE `cdzRustRun` + `rustRlibs` + the ambient
        # `rustc` (rustToolchain) it shells to compile the driver. NO compiler, so a compiler change cannot
        # rotate this beyond the (content-addressed) build input. `cdz-rust-run --grade` is the universal rust
        # grader (output/trap via compile+run; error/declines + warns from the captured compile outcome).
        mkCorpusRustExec = { name, build, idx }:
          pkgs.runCommand "corpus-rust-exec-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRustRun rustToolchain ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            mkdir -p "$TMPDIR/w"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --cdz-rt-dir ${rustRlibs} --cdz-num-dir ${rustRlibs} --cadenza-ast-dir ${rustRlibs}
                  --baseline ${./spec/semantics/.gate-baseline-rust}
                  --workdir "$TMPDIR/w")
            if [ -e ${build}/emit.rs ]; then args+=(--module ${build}/emit.rs); fi
            cdz-rust-run "''${args[@]}"
            echo "ok: corpus-rust ${name} case ${idx}" > "$out"
          '';

        corpusRustCaseChecks = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in
          builtins.listToAttrs (map
            (idx: {
              name = "${name}-${idx}";
              value = mkCorpusRustExec {
                inherit name idx;
                build = mkCorpusRustBuild { inherit name shred idx; };
              };
            })
            idxs);

        mkCorpusRustFileAgg = { name, file }:
          let cases = corpusRustCaseChecks { inherit name file; };
          in
          assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "corpus-rust-${name}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: corpus-rust ${name} — ${toString (builtins.length (builtins.attrNames cases))} cases via per-case shred→rust-build→rust-exec" > "$out"
          '';

        corpusRustFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            {
              name = "corpus-rust-${stem}";
              value = mkCorpusRustFileAgg { name = stem; file = ./spec/semantics + "/${f}"; };
            })
          corpusFileNames);
        corpusRustAll = pkgs.runCommand "corpus-rust-all" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues corpusRustFileAggs)}
          echo "ok: corpus-rust — ${toString (builtins.length corpusFileNames)} files graded via the per-case shred→rust-build→rust-exec caching graph" > "$out"
        '';

        # ── The RUST-ASYNC exec layer — the async/gas-metered rust backend twin of `corpus-rust` ──────────────
        # Closes the last native-only corpus target: `cargo xtask gate --target rust-async` runs IN-PROCESS
        # today because rust-async had NO cached nix check (main.rs: "rust-async … no cached check … stay
        # in-process"). Same per-case shred→build→exec graph as `corpus-rust`, but the build emits
        # `-t rust-async` and the exec grades with `--async` (links `cdz_rt`, reads the async signature marker)
        # against `.gate-baseline-rust-async`. Content-addressed build + compiler-free exec, exactly like the
        # sync rust layer — so a compiler change with identical async emit cache-hits every async exec.
        mkCorpusRustAsyncBuild = { name, shred, idx }:
          pkgs.runCommand "corpus-rust-async-build-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            case=$(echo ${shred}/${name}/${idx}-*)
            [ -d "$case" ] || { echo "no shred dir for case ${idx} of ${name}"; exit 1; }

            inputs=("ast:main=$case/program.ast")
            entry=()
            for m in "$case"/module-*.ast; do
              if [ -e "$m" ]; then
                n=$(basename "$m" .ast); n=''${n#module-}
                inputs+=("ast:$n=$m")
                entry=(--entry main)
              fi
            done

            if cdz-compile "''${inputs[@]}" "''${entry[@]}" -t rust-async -o "$out/emit.rs" 2>"$out/compile.err"; then
              printf '0' > "$out/compile.status"
            else
              printf '%s' "$?" > "$out/compile.status"
            fi
            cp "$case/test-run.ast" "$out/test-run.ast"
          '';

        mkCorpusRustAsyncExec = { name, build, idx }:
          pkgs.runCommand "corpus-rust-async-exec-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRustRun rustToolchain ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            mkdir -p "$TMPDIR/w"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --async --compile-status "$status" --compile-diag ${build}/compile.err
                  --cdz-rt-dir ${rustRlibs} --cdz-num-dir ${rustRlibs} --cadenza-ast-dir ${rustRlibs}
                  --baseline ${./spec/semantics/.gate-baseline-rust-async}
                  --workdir "$TMPDIR/w")
            if [ -e ${build}/emit.rs ]; then args+=(--module ${build}/emit.rs); fi
            cdz-rust-run "''${args[@]}"
            echo "ok: corpus-rust-async ${name} case ${idx}" > "$out"
          '';

        corpusRustAsyncCaseChecks = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in
          builtins.listToAttrs (map
            (idx: {
              name = "${name}-${idx}";
              value = mkCorpusRustAsyncExec {
                inherit name idx;
                build = mkCorpusRustAsyncBuild { inherit name shred idx; };
              };
            })
            idxs);

        mkCorpusRustAsyncFileAgg = { name, file }:
          let cases = corpusRustAsyncCaseChecks { inherit name file; };
          in
          assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "corpus-rust-async-${name}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: corpus-rust-async ${name} — ${toString (builtins.length (builtins.attrNames cases))} cases via per-case shred→rust-async-build→rust-async-exec" > "$out"
          '';

        corpusRustAsyncFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            {
              name = "corpus-rust-async-${stem}";
              value = mkCorpusRustAsyncFileAgg { name = stem; file = ./spec/semantics + "/${f}"; };
            })
          corpusFileNames);
        corpusRustAsyncAll = pkgs.runCommand "corpus-rust-async-all" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues corpusRustAsyncFileAggs)}
          echo "ok: corpus-rust-async — ${toString (builtins.length corpusFileNames)} files graded via the per-case shred→rust-async-build→rust-async-exec caching graph" > "$out"
        '';

        # VANISHED-check (gap #7 completion) — the GLOBAL half of baseline regression detection the per-case
        # exec cannot do. The per-case `--baseline` check catches a `pass -> not-pass` regression on a case
        # that RAN; it cannot see a baseline case that is no longer in the corpus at all (silently dropped,
        # its expected verdict now unenforced). This aggregate diffs each committed baseline's description
        # set against the corpus's — `cdz-corpus records` emits a `case\t<description>` line per case — and
        # FAILS on any baseline description absent from the corpus. The corpus description set is
        # backend-independent (every case runs on every backend), so ONE check covers all three baselines.
        # Closure = the parser (`cdzCorpus`) only; reruns when the corpus or a baseline changes.
        corpusVanishedCheck = pkgs.runCommand "corpus-vanished-check"
          { nativeBuildInputs = [ cdzCorpus ]; } ''
          set -euo pipefail
          # The corpus's case descriptions (from the flat record stream), sorted + unique.
          cdz-corpus records ${
            pkgs.lib.concatMapStringsSep " " (f: "${./spec/semantics + "/${f}"}") corpusFileNames
          } | grep '^case	' | cut -f2- | LC_ALL=C sort -u > corpus-descs
          rc=0
          for base in ${./spec/semantics/.gate-baseline} ${./spec/semantics/.gate-baseline-rust} ${./spec/semantics/.gate-baseline-rust-async}; do
            grep -v '^#' "$base" | grep -v '^$' | cut -f2- | LC_ALL=C sort -u > base-descs
            # Baseline descriptions with NO corpus case (`comm -23` = lines only in the first file).
            vanished=$(LC_ALL=C comm -23 base-descs corpus-descs || true)
            if [ -n "$vanished" ]; then
              echo "VANISHED baseline cases (in $(basename "$base"), no corpus case):" >&2
              echo "$vanished" >&2
              rc=1
            fi
          done
          if [ "$rc" -ne 0 ]; then exit 1; fi
          echo "ok: corpus-vanished — every committed baseline case still has a corpus case (3 baselines)" > "$out"
        '';

        # Full-CI-in-nix increment 6b: the GHA `codegen` job (`cargo xtask codegen --check`). This is the
        # runtime-ABI STALENESS gate: xtask regenerates runtime_abi.rs (+ wasm_abi.rs) — reading the
        # runtime WIT + BUILDING the cdz-runtime (release + debug) and cdz-nfc components via
        # `cargo component build` to fold in their content hashes (build_component_with_features) — and
        # `--check` fails if the committed file drifted. So it's the heaviest check: it needs BOTH the root
        # workspace (to build+run xtask) AND the runtime-component build machinery (cargo-component,
        # build-std via RUSTC_BOOTSTRAP, wasm-tools, the NFC WIT dep). It also runs rustfmt on the
        # generated file. NOTE: my `*-hash-parity` checks already assert the runtime hash == the committed
        # constant; codegen --check ADDS the full ABI-table regeneration guard.
        #   aarch64 MANDATORY — it builds the runtime whose content hash is arch-specific (same reason as
        #   the GHA codegen/gate jobs run on ubuntu-24.04-arm).
        codegenVendor = pkgs.symlinkJoin {
          name = "cdz-codegen-cargo-vendor";
          paths = [
            # xtask + the seed workspace (root lock).
            (pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; })
            # the cdz-runtime + cdz-nfc component builds (own locks) that codegen spawns.
            (pkgs.rustPlatform.importCargoLock {
              lockFile = ./implementation/seed/crates/cdz-runtime/Cargo.lock;
            })
            (pkgs.rustPlatform.importCargoLock {
              lockFile = ./implementation/seed/crates/cdz-nfc/Cargo.lock;
            })
            # build-std's own lockfile (core/alloc/panic_abort), shipped inside the pinned toolchain.
            (pkgs.rustPlatform.importCargoLock {
              lockFile = "${rustToolchain}/lib/rustlib/src/rust/library/Cargo.lock";
            })
          ];
        };
        codegenSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            # xtask + all root-workspace crates (xtask is a root member; codegen reads the runtime WIT).
            ./implementation/seed/crates
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
          ];
        };
        # CRANE-CONVERTED (v-nix, operator throughput 2026-08-09): consumes the merged-vendor release
        # dep-cache (cargoArtifactsReleaseCodegen) so a rotation recompiles only first-party, not the release
        # dep closure — same lever as gate-check. codegenVendor (the merged 4-lock superset) because codegen
        # builds the cdz-runtime + cdz-nfc components via cargo-component (needs those locks + build-std).
        # RUSTC_BOOTSTRAP + cargo-component preserved. Behavior UNCHANGED: same `cargo run -p xtask -- codegen
        # --check` (regenerates runtime_abi.rs, fails on drift).
        codegenCheck = craneLib.mkCargoDerivation {
          pname = "cdz-codegen-check";
          version = "0.0.0";
          src = codegenSrc;
          cargoArtifacts = cargoArtifactsReleaseCodegen;
          cargoVendorDir = codegenVendor;
          CARGO_PROFILE = "release";
          RUSTC_BOOTSTRAP = "1";
          doInstallCargoArtifacts = false;
          nativeBuildInputs = [ pkgs.wasm-tools pkgs.cargo-component ];
          # cargo-component writes a cache under $HOME/$XDG_CACHE_HOME; crane's mkCargoDerivation doesn't set
          # a writable HOME (the old stdenv buildPhase did via mkCargoVendorEnv), so point both at $TMPDIR or
          # cargo-component fails "Unable to create cache directory (Permission denied)".
          preBuild = ''
            export HOME="$TMPDIR/home"
            export XDG_CACHE_HOME="$TMPDIR/cache"
            mkdir -p "$HOME" "$XDG_CACHE_HOME"
          '';
          # xtask codegen --check regenerates runtime_abi.rs (building cdz-runtime + cdz-nfc components via
          # cargo-component to fold in their hashes) and fails if the committed file drifted. --locked =
          # hard-fail on root-lockfile drift.
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- codegen --check
          '';
          installPhaseCommand = ''
            echo "ok: cdz-codegen-check (cargo xtask codegen --check, crane release-deps-cached)" > "$out"
          '';
        };

        # Full-CI-in-nix increment 6c: the GHA `gate` job — THE behavior gate. `cargo xtask gate --check`
        # compiles + runs every corpus case (spec/semantics/*.sexp) through the cdz-syntax→rcdzc→cdz-run
        # pipeline, composing each program with the value-heap runtime, and grades the outcome vs the
        # committed `.gate-baseline*` — failing on a REGRESSION. The CI job runs `xtask build` first to
        # populate target/cadenza-store; here we SKIP that by pointing `--store` at my `componentStore`
        # derivation (already the content-addressed runtime store), so the check reuses the nix-built
        # components instead of rebuilding them. Needs: the seed workspace (to build cdz/rcdzc/cdz-run) +
        # spec/semantics (corpus + baselines) + wasm-tools (composition) + the store. aarch64 MANDATORY
        # (it composes the runtime whose hash is arch-specific). REQUIRED-class job.
        gateSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
            # the corpus `.sexp` + the `.gate-baseline*` files the gate grades against.
            ./spec/semantics
          ];
        };
        # CRANE-CONVERTED (v-nix, operator throughput 2026-08-09): consumes the RELEASE dep-cache
        # (cargoArtifactsRelease) via craneLib.mkCargoDerivation so crane restores the release deps' target/
        # before the run — a rotation recompiles only FIRST-PARTY (rcdzc/cdz/xtask/…), NOT the ~55 release
        # deps (measured: a corpus-only .sexp edit was ~330s of which the dep closure is the bulk). Gate only
        # builds the native host binaries (cdz/rcdzc/cdz-run — no build-std, no component builds), so the
        # ROOT lock (seedCargoVendor) covers it; codegenVendor's extra runtime/nfc/build-std locks were
        # over-provisioning for gate, and cargoArtifactsRelease is built with seedCargoVendor so the restored
        # target/ matches. Behavior UNCHANGED: same `cargo run -p xtask -- gate --check --store` command,
        # same corpus grading against the committed baselines — only the dep-compile is now cached.
        gateCheck = craneLib.mkCargoDerivation {
          pname = "cdz-gate-check";
          version = "0.0.0";
          src = gateSrc;
          cargoArtifacts = cargoArtifactsRelease;
          cargoVendorDir = seedCargoVendor;
          CARGO_PROFILE = "release";
          doInstallCargoArtifacts = false;
          nativeBuildInputs = [ pkgs.wasm-tools ];
          # Grade the whole corpus against the committed baselines, resolving the runtime from my nix-built
          # component store (skips the CI job's `xtask build`). --locked = hard-fail on lock drift.
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- gate --check --store "${componentStore}"
          '';
          installPhaseCommand = ''
            echo "ok: cdz-gate-check (cargo xtask gate --check --store <nix store>, crane release-deps-cached)" > "$out"
          '';
        };

        # gateCheckRust — the RUST-BACKEND gate, a NARROW per-MR subset (v-nix+v-ft 2026-08-10). WHY: gateCheck
        # above runs `gate --check` with NO --target → it defaults to WASM, so the RUST backend emit was NEVER
        # gated in localGate. A rust-only divergence (v-effects E0425: mutual-recursive effect-spec dedup dropped
        # a by-name-resolved fn — green-on-wasm, red-on-rust) reached trunk green through the wasm-only gate.
        # This folds a rust-backend check into localGate to close that hole. SUBSET, NOT FULL: `--target rust`
        # is a rustc-invocation-per-case (measured: even a tiny slice ~76s cold; the full 6686-case baseline is
        # >1hr of rustc) — a full per-MR rust gate would re-serialize pr-sync exactly like the corpus-over-
        # trigger did. So `--case mutual` (the 38 mutual-recursion cases = the EXACT divergence-mechanism class
        # that bit v-effects, NOT the whole ~1260-case 14-effects file) is the tight principled needle: it runs
        # the mutual-rec emit through rustc where the by-name-resolution divergence surfaces. VERIFIED the needle
        # runs those cases through the rust backend + passes on the fixed trunk (so the pre-fix shape would have
        # RED'd here). Full rust-backend coverage of the classes OUTSIDE the needle comes from a NIGHTLY
        # scheduled `gate --check --target rust` (checks.yml, NOT a localGate constituent) — widen the per-MR
        # needle reactively when a new divergence class reds nightly (same widen-on-new-class pattern as a
        # baseline). Rotates on a compiler-closure edit (seedCompiler in the emit path) → reruns when rust emit
        # can diverge; caches otherwise. Case-set policy is v-ft's; the derivation + fold are mine.
        gateCheckRust = craneLib.mkCargoDerivation {
          pname = "cdz-gate-check-rust";
          version = "0.0.0";
          src = gateSrc;
          cargoArtifacts = cargoArtifactsRelease;
          cargoVendorDir = seedCargoVendor;
          CARGO_PROFILE = "release";
          doInstallCargoArtifacts = false;
          # rustToolchain: the rust backend emits a Rust source artifact + compiles it with rustc per case, so
          # the toolchain must be on PATH (unlike the wasm gate). wasm-tools kept for parity with gateCheck's
          # pipeline needs.
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools ];
          # --target rust drives each matched case through the Rust backend (emit → rustc → run), graded against
          # .gate-baseline-rust. --case mutual = the narrow divergence-prone needle (see the note above).
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- gate --check --target rust --case "mutual" --store "${componentStore}"
          '';
          installPhaseCommand = ''
            echo "ok: cdz-gate-check-rust (gate --check --target rust --case mutual — narrow rust-backend divergence guard)" > "$out"
          '';
        };

        # Full-CI-in-nix increment 6d: the GHA `bench` job (`cargo xtask bench`) — the runtime ALLOCATION
        # benchmark. xtask runs cdz-runtime's `#[ignore]`d `hot_op_allocation_ceilings` test
        # (`cargo test --release … --ignored --test-threads=1`), parses its ALLOC lines, and diffs vs the
        # committed `spec/bench/.alloc-baseline` — failing on a regression. It's a NATIVE (host) cargo test
        # — cdz-runtime's `.cargo/config.toml` scopes build-std to `[target.wasm32-unknown-unknown]` ONLY,
        # so the host test needs NO build-std / RUSTC_BOOTSTRAP / wasm-tools (simpler than codegen/gate).
        # REQUIRED-class job. Runs on aarch64 here (the flake's host); the ALLOCATION COUNT is
        # arch-independent (it's gross heap allocs, not wall-clock or codegen), so this matches the GHA
        # bench job's x86_64 (ubuntu-latest) run against the same committed baseline (github-liaison #2042).
        benchSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
            # the committed allocation baseline the bench diffs against.
            ./spec/bench
          ];
        };
        # CRANE-CONVERTED (v-nix, operator throughput 2026-08-09): consumes the merged-vendor release
        # dep-cache (cargoArtifactsReleaseCodegen) so a rotation recompiles only first-party, not the release
        # dep closure — same lever as gate-check, applied to the bench check. codegenVendor (not
        # seedCargoVendor) because the bench test compiles against the cdz-runtime lock. Behavior UNCHANGED:
        # same `cargo run -p xtask -- bench` diffing cdz-runtime's hot_op_allocation_ceilings vs
        # spec/bench/.alloc-baseline.
        benchCheck = craneLib.mkCargoDerivation {
          pname = "cdz-bench-check";
          version = "0.0.0";
          src = benchSrc;
          cargoArtifacts = cargoArtifactsReleaseCodegen;
          cargoVendorDir = codegenVendor;
          CARGO_PROFILE = "release";
          doInstallCargoArtifacts = false;
          nativeBuildInputs = [ ];
          RUST_MIN_STACK = "67108864";
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- bench
          '';
          installPhaseCommand = ''
            echo "ok: cdz-bench-check (cargo xtask bench, crane release-deps-cached)" > "$out"
          '';
        };

        # Full-CI-in-nix increment 6f: the GHA `guide-examples` job — the guide's runnable-content gate
        # (`cargo xtask guide-wasm` then, in guide/, `npm ci` + a dozen `npm run check:*` + `npm run build`
        # + check:bundle). This is the LAST required-set job to nixify (ruleset 10560470) and the heaviest:
        # it composes the browser compiler wasm + a node toolchain + the value-heap runtime store.
        #
        #   aarch64 MANDATORY (like gate/codegen/bench): the guide bundles the value-heap runtime whose
        #   content hash the compiler pins (staged via the runtime hash embedded in cdz_wasm) — reproducible
        #   per-arch only. The context name is `guide examples` (no platform promise), so aarch64 is fine.
        #
        # HERMETIC wasm-pack: `wasm-pack build` network-downloads its own wasm-bindgen + wasm-opt at run time
        # (fatal in a sandbox), so we REPLICATE what it does with pinned nix tools instead: `cargo build
        # --target wasm32-unknown-unknown --release` in cdz-wasm, then the wasm-bindgen CLI (`--target web`)
        # over the cdylib to emit pkg/ (the JS glue + cdz_wasm_bg.wasm the guide imports), then wasm-opt -Os
        # (the release-profile shrink wasm-pack applies). cdz-wasm is its OWN root-excluded [workspace]
        # (path-deps rcdzc → the 7-crate compiler closure), vendoring from its OWN committed leaf lock
        # (`cdz-wasm/Cargo.lock`, 187 pkgs).
        #   wasm-bindgen CLI is version-LOCKED to the crate: `wasm-bindgen` in the leaf lock is 0.2.126, and
        #   the CLI schema-checks an EXACT version match, but nixpkgs ships 0.2.121 — so we build 0.2.126 via
        #   `buildWasmBindgenCli` (the same builder nixpkgs' own package uses), pinning the crate + vendor
        #   hashes. A crate bump means re-pinning these two hashes (discover by zeroing + reading `got:`).
        cdzWasmVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-wasm/Cargo.lock;
        };
        # The 0.2.126 wasm-bindgen CLI matching the leaf lock's `wasm-bindgen` crate (nixpkgs ships 0.2.121).
        wasmBindgenCli =
          let
            src = pkgs.fetchCrate {
              pname = "wasm-bindgen-cli";
              version = "0.2.126";
              hash = "sha256-H6Is3fiZVxZCfOMWK5dWMSrtn50VGv0sfdnsT+cTtyk=";
            };
          in
          pkgs.buildWasmBindgenCli {
            inherit src;
            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              inherit src;
              inherit (src) pname version;
              hash = "sha256-VucqkXbCi4qtQzY/HrXiDnbSURsagPsdNVMn1Tw3UiY=";
            };
          };
        # The vendored npm dependency set for guide/ (fixed-output; hash off guide/package-lock.json — a
        # dependency change re-pins via `nix run nixpkgs#prefetch-npm-deps -- guide/package-lock.json`).
        guideNpmDeps = pkgs.fetchNpmDeps {
          # fetchNpmDeps reads package-lock.json at the source ROOT — root the fileset at ./guide so the
          # lock lands at top level (a fileset rooted at ./. would nest it under guide/ and the builder
          # errors "No lock file!"). Scoped to just the lock → the vendor re-derives only on a lock change.
          src = pkgs.lib.fileset.toSource {
            root = ./guide;
            fileset = ./guide/package-lock.json;
          };
          hash = "sha256-BDmtWCGFSZ9iTkStHBFT/otezwwZGZEQIBBRHyHdrrM=";
        };
        # The guide source + the cdz-wasm compiler closure + the staged-lib sources (CAD + music) the
        # preload checks read. NOT the whole repo (fine-grained cache). NB: the CAD/music `.cdz` libs live
        # under implementation/{cad,music}/src — stage-wasm.mjs copies them into guide/src/wasm/{cad,music}.
        guideExamplesSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (map (c: ./implementation/seed/crates + ("/" + c)) [
              "cdz-wasm" "rcdzc" "cadenza-syntax" "cadenza-ast" "cdz-run" "cdz-rt" "cdz-num"
            ]) ++ [
              ./guide
              ./implementation/cad/src
              ./implementation/music/src
              ./rust-toolchain.toml
            ]
          );
        };
        # The browser compiler wasm `pkg/` (cdz_wasm_bg.wasm + the wasm-bindgen JS glue) as its OWN
        # INPUT-ADDRESSED derivation. WHY EXTRACT: guideExamplesCheck built this INLINE, so its src fileset
        # (correctly) includes the compiler crate closure (cdz-wasm → rcdzc → cadenza-syntax/ast/cdz-run/
        # cdz-rt/cdz-num); pulling the wasm build into its own derivation keeps the two disjoint filesets
        # separate (this is compiler crates only, NOT ./guide).
        #
        # WHY NOT FIXED-OUTPUT (reverted 2026-08-09, v-nix — was the operator sub-1-min throughput lever):
        # a FOD's output PATH is keyed on its hand-pinned outputHash, NOT its inputs, so nix serves the
        # CACHED output without rebuilding whenever that path already exists. But cdz_wasm_bg.wasm embeds
        # REQUIRED_RUNTIME_HASH (cdz-wasm required_runtime_hash() → rcdzc runtime_abi.rs). On a
        # REQUIRED_RUNTIME_HASH bump the wasm bytes legitimately change while the pinned outputHash did NOT,
        # so a WARM local-gate served the STALE wasm (still embedding the old hash) while componentStore
        # rebuilt to the new one. stage-wasm.mjs then scanned the stale wasm, looked for the OLD runtime hash
        # in componentStore (which has only the NEW one), found nothing, skipped staging → guide/src/wasm/
        # runtime.wasm absent → check:examples ENOENT → local-gate RED. The "fails loud on a bad pin" guard
        # only fires on a COLD build (no cached output to serve); warm it fails SILENTLY-stale. This blocked
        # EVERY REQUIRED_RUNTIME_HASH-bumping MR fleet-wide — the first such bumps since the GHA→local-gate
        # cutover (v-runtime B0, v-syntax record-render) — which is why it surfaced only now. Input-addressed
        # tracks the compiler source (rcdzc IS in the fileset), so a hash bump rotates this derivation →
        # rebuilds the wasm → stages the new runtime → green. COST: a compiler-only edit no longer
        # early-cutoffs the npm battery. Restore that later via a floating content-addressed derivation
        # (__contentAddressed = true) if the fleet enables ca-derivations — CA keys the path on the ACTUAL
        # built bytes, giving byte-identical-output early-cutoff AND a correct rebuild-on-bump, which a fixed
        # pin structurally cannot.
        guideCompilerWasmSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (map (c: ./implementation/seed/crates + ("/" + c)) [
              "cdz-wasm" "rcdzc" "cadenza-syntax" "cadenza-ast" "cdz-run" "cdz-rt" "cdz-num"
            ]) ++ [ ./rust-toolchain.toml ]);
        };
        cdzWasmPkg = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-wasm-pkg";
          version = "0.0.0";
          src = guideCompilerWasmSrc;
          nativeBuildInputs = [ rustToolchain wasmBindgenCli pkgs.binaryen ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = cdzWasmVendor; }}
            # REMAP the vendored-crate + toolchain store paths out of the embedded panic/debug location
            # strings. Without this the release wasm bakes absolute /nix/store/…-cargo-vendor-dir/… and
            # …-rust-…/… paths into its debug strings, which makes the output non-reproducible (the
            # vendor-dir hash rotates). Remapping to stable placeholders strips that — the emitted wasm is
            # then byte-stable across vendor/toolchain rotations. (Kept from the prior FOD form, where it was
            # also mandatory for store-path-free output; now it just preserves determinism + a stable path,
            # and is a prerequisite for the future content-addressed early-cutoff noted above.)
            export RUSTFLAGS="--remap-path-prefix=$(${pkgs.coreutils}/bin/readlink -f ${cdzWasmVendor})=vendor --remap-path-prefix=${rustToolchain}=rust ''${RUSTFLAGS:-}"
            ( cd implementation/seed/crates/cdz-wasm
              cargo build --release --target wasm32-unknown-unknown --locked
              wasm-bindgen --target web --out-dir pkg \
                target/wasm32-unknown-unknown/release/cdz_wasm.wasm
              # wasm-pack's --release runs wasm-opt; the crate profile is opt-level="s" → -Os.
              wasm-opt -Os pkg/cdz_wasm_bg.wasm -o pkg/cdz_wasm_bg.wasm
            )
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp -r implementation/seed/crates/cdz-wasm/pkg "$out"
            runHook postInstall
          '';
        };
        guideExamplesCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-guide-examples";
          version = "0.0.0";
          src = guideExamplesSrc;
          nativeBuildInputs = [
            rustToolchain
            pkgs.nodejs_22 # node 22 (>=22.6 for --experimental-strip-types in test:unit)
            pkgs.npmHooks.npmConfigHook # wires npmDeps → the offline npm cache (npm ci runs offline)
          ];
          # npmConfigHook reads these: the vendored dep set + the dir holding package-lock.json.
          npmDeps = guideNpmDeps;
          npmRoot = "guide";
          # The base the guide's vite build fingerprints assets under — mirror the GHA env
          # (VITE_BASE=/<repo>/). The repo is `cadenza`; a bundle-path check reads it.
          VITE_BASE = "/cadenza/";
          buildPhase = ''
            runHook preBuild

            # ── 1. Consume the pre-built browser compiler wasm pkg/ (cdzWasmPkg, an input-addressed
            # derivation keyed on the compiler crate closure — rotates on a compiler/runtime-hash edit so the
            # staged runtime always matches the embedded hash). Copy it where stage-wasm.mjs expects it.
            cp -r ${cdzWasmPkg} implementation/seed/crates/cdz-wasm/pkg
            chmod -R u+w implementation/seed/crates/cdz-wasm/pkg

            # ── 2. Stage pkg/ + the value-heap runtime + the CAD/music preload libs into guide/src/wasm/.
            # stage-wasm.mjs finds the runtime by the hash embedded in the compiler wasm, in CADENZA_STORE.
            export CADENZA_STORE="${componentStore}"
            node guide/scripts/stage-wasm.mjs

            # ── 3. The guide gate: install (offline, from the npm cache the hook wired), then the exact
            # check sequence checks.yml runs (unit → prose → diagnostics → examples → calculator → the
            # conformance guards → build → bundle). Same order so a failure maps 1:1 to the GHA job.
            ( cd guide
              npm ci
              # The vendored bins (tsc, vite, …) ship `#!/usr/bin/env node` shebangs; /usr/bin/env doesn't
              # exist in the hermetic sandbox → rewrite to nix paths. Patch the WHOLE tree, not just
              # node_modules/.bin: those are symlinks (find -type f skips them), the real files live under
              # node_modules/<pkg>/bin/, so `.bin`-only leaves tsc/vite unpatched (build → bad interpreter).
              patchShebangs node_modules
              npm run test:unit
              npm run check:prose
              npm run check:diagnostics
              npm run check:examples
              npm run check:calculator
              npm run check:worker-stack
              npm run check:tuple-collection
              npm run check:cad-preload
              npm run check:music-preload
              npm run build
              npm run check:bundle
            )
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-guide-examples (wasm-pack + npm ci + check:* + build + bundle)" > "$out"
            runHook postInstall
          '';
        };

        # ── oracle-lean (L0.1): the Lean reference interpreter as an independent differential oracle ─
        #
        # A pure Lean 4 model of Cadenza semantics over the frozen binary AST, cross-checked against
        # rcdzc to scale compiler bug-finding (design: implementation/design/DESIGN-lean-differential-
        # oracle.md). Its OWN Lake project under implementation/oracle-lean/ (mirrors cdz-smith being
        # its own workspace) — the `cdz-oracle` exe reads a request frame on stdin + writes verdicts on
        # stdout. It is Lean-stdlib-only (no `lake` deps), so this build is hermetic — no network fetch.
        #
        # `pkgs.stdenv` (NOT stdenvNoCC): Lean's `leanc` links the compiled exe with a C compiler, so
        # the derivation needs the matching nix `cc`/`ld` on PATH — building outside a nix stdenv links
        # against the host glibc + `/usr/bin/ld` and fails on `__isoc23_*` symbol mismatches.
        # `LEAN_ABORT_ON_PANIC=1` keeps a Lean panic a hard failure. The fileset is enumerated (never
        # the `.lake` build dir) so a local `lake build` can't leak stale artifacts into the sandbox.
        oracleLeanSrc = pkgs.lib.fileset.toSource {
          root = ./implementation/oracle-lean;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/oracle-lean/lakefile.toml
            ./implementation/oracle-lean/lean-toolchain
            ./implementation/oracle-lean/Oracle.lean
            ./implementation/oracle-lean/Main.lean
            ./implementation/oracle-lean/OracleTest.lean
            ./implementation/oracle-lean/OracleAstTest.lean
            ./implementation/oracle-lean/OracleCheck.lean
            ./implementation/oracle-lean/Oracle
          ];
        };
        oracleLean = pkgs.stdenv.mkDerivation {
          pname = "cdz-oracle-lean";
          version = "0.0.0";
          src = oracleLeanSrc;
          nativeBuildInputs = [ pkgs.lean4 ];
          buildPhase = ''
            runHook preBuild
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            # fileset.toSource copies are read-only; lake writes .lake/ into the tree.
            chmod -R u+w .
            lake build cdz-oracle oracle-selftest oracle-ast-roundtrip oracle-check
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p "$out/bin"
            install -m755 .lake/build/bin/cdz-oracle "$out/bin/cdz-oracle"
            install -m755 .lake/build/bin/oracle-selftest "$out/bin/oracle-selftest"
            install -m755 .lake/build/bin/oracle-ast-roundtrip "$out/bin/oracle-ast-roundtrip"
            install -m755 .lake/build/bin/oracle-check "$out/bin/oracle-check"
            runHook postInstall
          '';
        };
        # L0.1 gate witness: build the oracle + run the self-test (a smoke request round-trips through
        # the frame codec + the declining handler, yielding one Unsupported verdict). Non-zero exit
        # from the self-test fails the derivation.
        oracleLeanSmoke = pkgs.runCommand "oracle-lean-smoke"
          { nativeBuildInputs = [ oracleLean ]; } ''
          oracle-selftest
          echo "ok: oracle-lean smoke — cdz-oracle builds + selftest round-trips (Unsupported)" > "$out"
        '';
        # L0.2 gate witness: the binary-AST codec is byte-identical on EVERY corpus-derived
        # `program.ast` blob (decode → re-encode == input). Fixtures are NOT committed and NOT
        # re-shredded (operator 2026-08-27): this REUSES the corpus pipeline's existing per-file
        # `mkCorpusShred` derivations (identical args → identical store path → cache hit, no extra
        # shred work) and aggregates their `program.ast` outputs into one round-trip. Each shred is the
        # canonical `codec::encode` of a `spec/semantics/*.sexp` case (cadenza syntax → binary AST), so
        # decode∘encode must be the identity — a codec-law check of the oracle's own decoder, not a
        # re-test of corpus semantics (PRINCIPLES.md §2). Non-zero exit (decode error / byte mismatch)
        # fails. `oracleLeanShreds` maps the SAME `mkCorpusShred { name = stem; file }` the corpus
        # aggregates use, so the shred cache is shared fleet-wide.
        oracleLeanShreds = map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            mkCorpusShred { name = stem; file = ./spec/semantics + "/${f}"; })
          corpusFileNames;
        # The corpus CASE-DIR manifest, computed ONCE (operator 2026-08-27: don't recompute corpus
        # locations in every check). One line per shredded case dir (each holds `program.ast` +
        # `oracle-trial.ast`), sorted; both the round-trip and the conformance checks consume this file
        # in place (never re-`find`). Keyed on the shreds, so it rotates only when the corpus changes.
        oracleLeanCaseDirs = pkgs.runCommand "oracle-lean-case-dirs"
          { shreds = oracleLeanShreds; } ''
          for s in $shreds; do find "$s" -name oracle-trial.ast; done | sed 's|/oracle-trial.ast$||' | sort > "$out"
        '';
        oracleLeanAstRoundtrip = pkgs.runCommand "oracle-lean-ast-roundtrip"
          { nativeBuildInputs = [ oracleLean ]; caseDirs = oracleLeanCaseDirs; } ''
          # Derive the program.ast paths from the shared case-dir manifest (computed once), never a fresh find.
          sed 's|$|/program.ast|' "$caseDirs" > manifest
          echo "oracle-lean ast round-trip: $(wc -l < manifest) program.ast blobs (shared case-dir manifest)"
          oracle-ast-roundtrip --manifest manifest
          echo "ok: oracle-lean ast round-trip — binary-AST decode/encode byte-identical on $(wc -l < manifest) corpus program.ast blobs" > "$out"
        '';
        # L1.2: the corpus-conformance run — the oracle ASSERTS each corpus case. Consumes the shared
        # case-dir manifest (each dir holds `program.ast` + `oracle-trial.ast`, the latter emitted by the
        # shred, #4252): `oracle-check` evaluates the trials and asserts the expected outcome. A `mismatch`
        # on any realized trial is a real oracle-vs-corpus disagreement (fails); `skip`
        # (Unsupported/Diverges/compile-outcome) is a sound coverage-gap. The operator's quality signal
        # (corpus-conformance), keyed on {shred, lean oracle}. Standalone/advisory like the other checks.
        oracleLeanCheck = pkgs.runCommand "oracle-lean-check"
          { nativeBuildInputs = [ oracleLean ]; caseDirs = oracleLeanCaseDirs; } ''
          echo "oracle-lean check: $(wc -l < "$caseDirs") corpus cases (shared case-dir manifest)"
          oracle-check --manifest "$caseDirs" | tee result
          echo "ok: oracle-lean corpus conformance — $(cat result)" > "$out"
        '';
      in
      {
        # N1: the value-heap runtime components as NORMAL (input-addressed) derivations — `nix build
        # .#runtime` / `.#runtime-debug` builds + strips the wasm; `.#runtime-hash` / `.#runtime-debug-hash`
        # is the DERIVED content address (sha256 of the built bytes), the value a program pins. The hash
        # is never asserted here — it falls out of the build (operator north star). Parity with the
        # committed REQUIRED_RUNTIME_HASH is a `checks` assertion below, not a pin.
        packages.runtime = runtime;
        # R3 (v-nix+v-runtime 2026-08-09): the RAW pre-strip runtime wasm (cdz-abi custom section intact) —
        # the `raw` output of the SAME single build as `packages.runtime` (no extra rebuild). An R3 codegen
        # consumer reads cdz-abi from here (read_abi_imm_unit) + hashes packages.runtime (stripped). Additive:
        # packages.runtime (stripped) is byte-unchanged, so exposing this does not move REQUIRED_RUNTIME_HASH.
        packages.runtime-raw = runtime.raw;
        packages.runtime-debug = runtimeDebug;
        # The `*-hash` outputs are the SHARED `hashOf` derivations (also consumed by componentStore + the
        # compiler-hash injection + the NFC-stamp), so `nix build .#runtime-hash` yields the exact file those
        # consumers `cat` — one hash derivation per component, not one per use-site.
        packages.runtime-hash = runtimeHash;
        packages.runtime-debug-hash = runtimeDebugHash;

        # N1: the NFC component (`cdz-nfc`) the runtime imports by hash (REQUIRED_NFC_HASH). `.#nfc` is
        # the stripped component; `.#nfc-hash` its derived content address.
        packages.nfc = nfc;
        packages.nfc-hash = nfcHash;

        # No per-reducer `packages.*` aliases (operator 2026-08-24 — no hardcoded reducer names): every
        # Cadenza guest is auto-enumerated in `harnessPrograms` + built by its `harness-<reducer>-echo` check
        # (`.#checks.<sys>.harness-reducer-…-cdz-echo`), so a hardcoded `.#reducer-…-cdz` alias would just
        # re-introduce the names the tree already derives. `.#world-artifacts` stays (the KIND_WIT_WORLD
        # binaries the host-import guests consume — not a reducer name).
        packages.world-artifacts = worldArtifacts;

        # The integration-test executable, built ONCE (§9) — `nix build .#cdz-platform-itest` →
        # result/bin/cdz-platform-itest. Shared by every harness run so a test/program change never rebuilds it.
        packages.cdz-platform-itest = platformItest;

        # The contract name→hash tooling (section 1). `.#cdz-contract` is the built CLI; `.#contract-hashes`
        # is the reproducible name→base62-id JSON mapping over the platform's contract sources — the data a
        # run resolves a `contract = "<name>"` reference against (see mkHarnessAst).
        packages.cdz-contract = contractHasher;
        packages.contract-hashes = contractHashes;

        # R2: the content-addressed component store — every nix-built component as `<derived-hash>.wasm`
        # in one dir (mirrors target/cadenza-store, but built + addressed by nix). `nix build .#store`.
        packages.store = componentStore;

        # crane MR1 (additive): the cached dependency-compile layer for the per-crate clippy/test checks.
        # `nix build .#cargo-artifacts` builds it; a main-branch run SAVES it to the shared /nix/store so
        # MR2/MR3's per-crate checks RESTORE it instead of recompiling the dep closure. No check consumes it
        # yet — this MR only proves it builds + warms the cache.
        packages.cargo-artifacts = cargoArtifacts;

        # S1: the native seed compiler (cdz + cdz-run). `nix build .#seed-compiler` → result/bin/{cdz,cdz-run}.
        packages.seed-compiler = seedCompiler;

        # oracle-lean (L0.1): the Lean differential oracle. `nix build .#oracle-lean` →
        # result/bin/{cdz-oracle,oracle-selftest}.
        packages.oracle-lean = oracleLean;

        # rcdzc→wasm: the compiler as a wasm artifact for the agent kernel's blob store. `.#rcdzc-wasm`
        # is the wasm module; `.#rcdzc-wasm-hash` its derived content address (for v-agent-harness's
        # compiler-latest store pointer).
        packages.cdz-wasm-pkg = cdzWasmPkg;
        packages.cargo-artifacts-release = cargoArtifactsRelease;
        packages.cargo-artifacts-release-codegen = cargoArtifactsReleaseCodegen;
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
        # check` time. `runtime_abi.rs` is `@generated by cargo xtask codegen`; each hash const is now
        # `= match option_env!("CDZ_…") { Some(h) => h, None => "<hash>" }` (the compile-time override, so a
        # nix build can inject the runtime it built — see `seedCompiler`), so the COMMITTED value is the
        # 45-char base62 literal in the `None =>` DEFAULT arm. We split on that arm marker and take the
        # leading 45 base62 chars (guarded: the marker MUST be present and the chars MUST be base62, else we
        # THROW rather than compare against a stray literal). Platform content address, §8 — not the old 64-hex.
        checks =
          let
            abi = builtins.readFile
              ./implementation/seed/crates/rcdzc/src/backend/wasm/runtime_abi.rs;
            recordedHash = constName:
              let
                decl = "pub const " + constName + ": &str =";
                parts = builtins.split decl abi;
                afterDecl = if builtins.length parts >= 3 then pkgs.lib.last parts else null;
                # The const's default arm: `None => "<45-char base62>"`. Split on the marker; the segment
                # after the FIRST occurrence (this const's own arm, since `afterDecl` begins at this decl)
                # starts with the hash, so its leading 45 chars are it.
                marker = "None => \"";
                afterMarker =
                  if afterDecl == null then null
                  else let seg = builtins.split marker afterDecl;
                       in if builtins.length seg >= 3 then pkgs.lib.elemAt seg 2 else null;
                hash = if afterMarker == null then null
                       else builtins.substring 0 45 afterMarker;
                valid = hash != null && builtins.match "[0-9A-Za-z]{45}" hash != null;
              in
              if afterDecl == null then
                throw "flake.nix: `${decl}` not found in runtime_abi.rs (codegen shape changed?)"
              else if !valid then
                throw "flake.nix: `${decl}` found but its `None =>` arm holds no 45-char base62 default literal"
              else hash;
            parity = { name, hashFile, constName }:
              pkgs.runCommand "${name}-hash-parity" { } ''
                got=$(cat ${hashFile})
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
            # cdz's check is WORKSPACE-SRC (concierge-confirmed 1a) — see the long note at its registration.
            # Both crane aggregates (clippy + test) depend on this for cdz's lint+test (cdz doesn't fit the
            # per-crate closure isolation — it builds the whole workspace).
            crateCdzCheck = cargoWorkspaceCheck {
              name = "cargo-crate-cdz";
              cargoCmd = "cargo build --workspace --locked && cargo clippy -p cdz --all-targets --locked -- -D warnings && cargo test -p cdz --locked";
              src = seedTestSrc;
              extraInputs = [ pkgs.git ];
            };
            # crane MR2: the CLIPPY half via crane (per-crate cargoClippy consuming the shared cargoArtifacts →
            # deps NOT recompiled each run). Each maker takes crate/extraSrc/extraInputs. cdz stays
            # workspace-src (crateCdzCheck, different shape — its clippy is inside cargoWorkspaceCheck).
            perCrateClippyCrane = {
              clippy-cadenza-ast = mkCrateClippyCrane { crate = "cadenza-ast"; };
              clippy-cadenza-syntax = mkCrateClippyCrane { crate = "cadenza-syntax"; extraSrc = [ ./spec/semantics ]; };
              clippy-cdz-calc = mkCrateClippyCrane { crate = "cdz-calc"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              clippy-cdz-component-rewrite = mkCrateClippyCrane { crate = "cdz-component-rewrite"; };
              clippy-cdz-contract = mkCrateClippyCrane { crate = "cdz-contract"; };
              clippy-cdz-corpus = mkCrateClippyCrane { crate = "cdz-corpus"; extraSrc = [ ./spec/semantics ]; };
              clippy-cdz-corpus-grade = mkCrateClippyCrane { crate = "cdz-corpus-grade"; };
              clippy-cdz-num = mkCrateClippyCrane { crate = "cdz-num"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              clippy-cdz-platform = mkCrateClippyCrane { crate = "cdz-platform"; };
              clippy-cdz-rt = mkCrateClippyCrane { crate = "cdz-rt"; };
              clippy-cdz-run = mkCrateClippyCrane { crate = "cdz-run"; extraSrc = [ ./implementation/compiler-ml ]; };
              clippy-cdz-rust-render = mkCrateClippyCrane { crate = "cdz-rust-render"; };
              clippy-cdz-rust-run = mkCrateClippyCrane { crate = "cdz-rust-run"; };
              clippy-cdz-world-artifact = mkCrateClippyCrane { crate = "cdz-world-artifact"; };
              clippy-rcdzc = mkCrateClippyCrane {
                crate = "rcdzc";
                extraSrc = [ ./spec/semantics ./implementation/compiler-ml ./implementation/seed/crates/cdz-runtime/src/bigint.rs ];
              };
              clippy-xtask = mkCrateClippyCrane { crate = "xtask"; extraSrc = [ ./spec/semantics ./implementation/compiler-ml ]; extraInputs = [ pkgs.git ]; };
            };
            # cdz's clippy stays in its workspace-src check (crateCdzCheck runs `cargo clippy -p cdz` inside).
            clippyCraneAggregate = pkgs.runCommand "cargo-clippy-crane-aggregate"
              (perCrateClippyCrane // { inherit crateCdzCheck; }) ''
              echo "ok: clippy aggregate — all per-crate crane cargoClippy checks + cdz (crane MR2)" > $out
            '';
            # per-crate TEST set — the SAME closure-isolated crates as perCrateClippyCrane (identical
            # crate/extraSrc/extraInputs), via mkCrateTestCrane instead of mkCrateClippyCrane. This REPLACES
            # the whole-workspace testCheck's re-run-everything behavior: a 1-crate edit reruns only that
            # crate's (+ dependents') test derivation, the rest cache-hit. cdz stays in crateCdzCheck (its
            # run_rust_cli tests are whole-workspace-integration — see that binding), so the union is
            # {per-crate tests} + cdz, matching `cargo test --workspace`. Coverage parity is asserted by
            # testCrateCoverageAssert (below) so a new workspace member can't silently escape the test set.
            perCrateTestCrane = {
              test-cadenza-ast = mkCrateTestCrane { crate = "cadenza-ast"; };
              test-cadenza-syntax = mkCrateTestCrane { crate = "cadenza-syntax"; extraSrc = [ ./spec/semantics ]; };
              test-cdz-calc = mkCrateTestCrane { crate = "cdz-calc"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              test-cdz-component-rewrite = mkCrateTestCrane { crate = "cdz-component-rewrite"; };
              test-cdz-contract = mkCrateTestCrane { crate = "cdz-contract"; };
              test-cdz-corpus = mkCrateTestCrane { crate = "cdz-corpus"; extraSrc = [ ./spec/semantics ]; };
              test-cdz-corpus-grade = mkCrateTestCrane { crate = "cdz-corpus-grade"; };
              test-cdz-num = mkCrateTestCrane { crate = "cdz-num"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              test-cdz-platform = mkCrateTestCrane { crate = "cdz-platform"; };
              test-cdz-rt = mkCrateTestCrane { crate = "cdz-rt"; };
              test-cdz-run = mkCrateTestCrane { crate = "cdz-run"; extraSrc = [ ./implementation/compiler-ml ]; };
              test-cdz-rust-render = mkCrateTestCrane { crate = "cdz-rust-render"; };
              test-cdz-rust-run = mkCrateTestCrane { crate = "cdz-rust-run"; };
              test-cdz-wasm-opt-gap = mkCrateTestCrane { crate = "cdz-wasm-opt-gap"; };
              test-cdz-world-artifact = mkCrateTestCrane { crate = "cdz-world-artifact"; };
              test-rcdzc = mkCrateTestCrane {
                crate = "rcdzc";
                extraSrc = [ ./spec/semantics ./implementation/compiler-ml ./implementation/seed/crates/cdz-runtime/src/bigint.rs ];
              };
              test-xtask = mkCrateTestCrane { crate = "xtask"; extraSrc = [ ./spec/semantics ./implementation/compiler-ml ]; extraInputs = [ pkgs.git ]; };
            };
            # COVERAGE-PARITY assert (concierge mandate — no test silently dropped vs `cargo test
            # --workspace`): the per-crate test crates PLUS cdz (crateCdzCheck) must EXACTLY equal the
            # workspace member set (rootCrateNames). A new workspace member that isn't given a per-crate test
            # derivation would silently escape the gate — this throws at EVAL (fails `nix flake check` + the
            # localGate build) if the union drifts from rootCrateNames, in either direction. Mirrors
            # crateClosureAssert's fail-loud discipline. `test-<crate>` → `<crate>` by stripping the prefix.
            testCrateCoverageAssert =
              let
                covered = (map
                  (n: builtins.substring 5 (builtins.stringLength n) n) # strip "test-"
                  (builtins.attrNames perCrateTestCrane)) ++ [ "cdz" ];
                missing = builtins.filter (c: !(builtins.elem c covered)) rootCrateNames;
                extra = builtins.filter (c: !(builtins.elem c rootCrateNames)) covered;
              in
              if missing != [ ] || extra != [ ] then
                throw ("flake.nix per-crate test coverage-parity: the per-crate test set + cdz must equal the "
                  + "workspace members. MISSING (a member with no per-crate test): ${builtins.toString missing}; "
                  + "EXTRA (a test crate not a workspace member): ${builtins.toString extra}. "
                  + "Add/remove a mkCrateTestCrane entry so the union matches rootCrateNames.")
              else
                pkgs.runCommand "cargo-test-crate-coverage-assert" { } ''
                  echo "ok: per-crate test set + cdz == workspace members (${builtins.toString rootCrateNames})" > $out
                '';
            # per-crate TEST aggregate — a single green/red over all per-crate tests + cdz (workspace-src).
            # This is what localGate folds in place of the whole-workspace testCheck: same coverage, but a
            # 1-crate edit reruns only that crate's test derivation (the operator 1-min-gate mandate). Depends
            # on testCrateCoverageAssert so the aggregate is red if the set ever drifts from the workspace.
            testCraneAggregate = pkgs.runCommand "cargo-test-crane-aggregate"
              (perCrateTestCrane // { inherit crateCdzCheck testCrateCoverageAssert; }) ''
              echo "ok: test aggregate — all per-crate crane cargoTest checks + cdz (per-crate-incremental)" > $out
            '';
            # 2-WAY CLIPPY SHARD (v-nix+v-fleet-tooling 2026-08-07, data-driven CI-speed): clippy is the sole
            # critical-path pole (~8.8m) and RUN-TIME-bound (queue ~0.1m / run ~8.9m — v-ft calm-window n=119),
            # so splitting it into 2 PARALLEL GHA jobs directly halves the pole for only +1 x86 slot/candidate.
            # These two sub-aggregates each force ~half the per-crate clippy checks, BALANCED BY BUILD WEIGHT (not
            # count): the 2 HEAVIEST units — rcdzc (spec+compiler-ml+bigint closure) and crateCdzCheck (the
            # whole-workspace `cargo clippy -p cdz`, builds every rlib) — are split ACROSS shards (rcdzc on A,
            # crate-cdz on B, each shard's anchor), with cdz-run/xtask (next-heaviest) both on B and the light
            # units filling A. See the REBALANCED note on clippyShardA below for the measured-imbalance history.
            # Both cache-HIT the shared cargoArtifacts (no dep recompile). Exposed as checks.<sys>.clippy-shard-{a,b};
            # checks.yml runs them as 2 parallel jobs.
            # The union {A} ∪ {B} == clippyCraneAggregate's set exactly (coverage parity — no clippy unit dropped).
            # clippyCraneAggregate (the old `clippy` context) is KEPT during the additive-first cutover (STEP 1);
            # v-ft flips the ruleset to require the shards (STEP 2), then the old `clippy` job is dropped (STEP 3).
            # REBALANCED 2-way (STEP 4, 2026-08-07): the initial split put BOTH heavies — rcdzc AND crate-cdz
            # (the whole-workspace `cargo clippy -p cdz`, which builds every rlib) — on shard A → A=7m31s vs
            # B=2m34s (CI, #2503), so wall-clock = A = 7.5m (only ~1.6m under the old 9m). Fix: split the two
            # heavies ACROSS shards — rcdzc anchors A, crate-cdz moves to B — so each shard carries exactly one
            # heavyweight. Union unchanged (still the same 11 units, 4+7); pure flake edit, SAME 2 required
            # contexts → NO ruleset change (v-ft: a 2-way rebalance keeps the contexts). Target: both ~4-5m.
            clippyShardA = pkgs.runCommand "cargo-clippy-shard-a"
              {
                inherit (perCrateClippyCrane) clippy-rcdzc clippy-cdz-num clippy-cdz-calc clippy-cadenza-syntax clippy-cdz-platform
                  clippy-cdz-component-rewrite clippy-cdz-contract;
              } ''
              echo "ok: clippy shard A — rcdzc + cdz-num + cdz-calc + cadenza-syntax + cdz-platform + cdz-component-rewrite + cdz-contract" > $out
            '';
            clippyShardB = pkgs.runCommand "cargo-clippy-shard-b"
              {
                inherit crateCdzCheck;
                inherit (perCrateClippyCrane)
                  clippy-cdz-run clippy-xtask clippy-cadenza-ast clippy-cdz-corpus clippy-cdz-rt clippy-cdz-rust-render;
              } ''
              echo "ok: clippy shard B — cdz (workspace) + cdz-run + xtask + cadenza-ast + cdz-corpus + cdz-rt + cdz-rust-render" > $out
            '';
            # flakeReproBackstop: the REPRODUCIBILITY-BACKSTOP subset — the checks the `nix-flake (advisory)`
            # CI job should run INSTEAD of a whole `nix flake check`. Data-driven CI-speed (operator standing
            # mandate + v-ft queue-wait ranking, 2026-08-05): `nix flake check` was the biggest runner-cost
            # (~48.6m advisory) because it rebuilds the UNION of ALL flake outputs — clippy/test/codegen/gate/
            # bench/guide/roundtrip/fmt/kernel+host-native/rcdzc-wasm/cad — i.e. it REDUNDANTLY re-runs the whole
            # required-job set (each already run individually WITH the /nix/store cache by its own required job),
            # gating nothing. That redundant rebuild is pure runner-waste + eats slots (queue-wait median 5.3m/
            # p90 12m → freeing slots cuts time-to-merge fleet-wide). This aggregate keeps ONLY the coverage the
            # advisory UNIQUELY provides — nothing else in checks.yml builds these:
            #   · the 3 end-to-end HASH-PARITY checks (nix-built component bytes' sha256 == the committed
            #     REQUIRED_RUNTIME_HASH / DEBUG_RUNTIME_HASH / REQUIRED_NFC_HASH). codegen enforces source-
            #     staleness natively, but NOT the through-nix hash-from-bytes reproduction — that's this.
            #   · the example project's @test suite run through nix.
            #   · the pure-eval closure-assert guard.
            # The advisory job runs `nix build .#checks.<sys>.flake-repro-backstop` (minutes, cache-warm) instead
            # of `nix flake check` (48m). Coverage of the required set is unchanged (those jobs still run); only
            # the redundant re-run is dropped. `nix flake check` locally/in devShell still builds everything.
            flakeReproBackstop = pkgs.runCommand "flake-repro-backstop"
              {
                inherit runtimeHashParity runtimeDebugHashParity nfcHashParity
                  contractHashesValid harnessRunsAll
                  exampleProjectTests crateClosureAssert;
              } ''
              echo "ok: flake reproducibility-backstop — hash-parity + component-validity + project-@tests + closure-assert" > $out
            '';
            # bindings the backstop aggregate references (kept as `let` so both the aggregate + the individual
            # `checks.*` attrs below share ONE derivation each — no rebuild).
            runtimeHashParity = parity { name = "runtime"; hashFile = runtimeHash; constName = "REQUIRED_RUNTIME_HASH"; };
            runtimeDebugHashParity = parity { name = "runtime-debug"; hashFile = runtimeDebugHash; constName = "DEBUG_RUNTIME_HASH"; };
            nfcHashParity = parity { name = "nfc"; hashFile = nfcHash; constName = "REQUIRED_NFC_HASH"; };
            # The contract name→hash mapping is well-formed: a non-empty JSON object whose every value is a
            # base62 contract-id (§8 text form — `[0-9A-Za-z]`, the one post-flag-day form; no hex/base64url).
            # Catches a silently-empty mapping (e.g. a contracts dir that stopped parsing) that a run not
            # referencing contracts by name would never surface. The harness runs that DO name a contract are
            # the functional check on top.
            contractHashesValid = pkgs.runCommand "contract-hashes-valid"
              { nativeBuildInputs = [ pkgs.jq ]; } ''
              set -euo pipefail
              n="$(jq 'length' ${contractHashes})"
              if [ "$n" -lt 1 ]; then echo "contract-hash mapping is empty" >&2; exit 1; fi
              # every value is a non-empty base62 string (no `_`/`-` — base62, not base64url)
              if ! jq -e 'to_entries | all(.value | type == "string" and test("^[0-9A-Za-z]+$"))' \
                   ${contractHashes} > /dev/null; then
                echo "contract-hash mapping has a non-base62 id:" >&2; cat ${contractHashes} >&2; exit 1
              fi
              echo "ok: contract-hash mapping well-formed ($n contract(s), all base62 ids)" > "$out"
            '';

            # The HARNESS-RUN checks (§9), the shape the operator asked for on #2994: iterate every `*.ml`
            # spec in the harness-runs directory and build ONE check derivation per run via `mkHarnessRun`
            # (the shared `platformItest` binary + the per-program wasm store, resolved by name). Fine-grained
            # caching falls out: each run's derivation inputs are exactly {the binary, the programs it uses,
            # its own spec}, so a spec edit reruns only that run, a program edit reruns only its users, and
            # neither rebuilds the binary. A run passes iff the executable exits 0 — the harness/its checker
            # makes the assertions about the log, not nix.
            harnessRunDir = ./implementation/seed/crates/cdz-platform/harness-runs;
            harnessRunChecks = pkgs.lib.mapAttrs'
              (fn: _:
                let base = pkgs.lib.removeSuffix ".ml" fn; in
                pkgs.lib.nameValuePair base (mkHarnessRun {
                  name = base;
                  specFile = harnessRunDir + "/${fn}";
                }))
              (pkgs.lib.filterAttrs (n: t: t == "regular" && pkgs.lib.hasSuffix ".ml" n)
                (builtins.readDir harnessRunDir));
            # An aggregate over all harness runs — folded into flake-repro-backstop so the reproducible-guest
            # + integ-executable + deterministic-bach e2e path is gated as one node (each run still cached
            # independently; the aggregate only depends on their pass markers).
            harnessRunsAll = pkgs.runCommand "harness-runs-all" { } ''
              # Depend on every run's pass-marker by reading it (interpolating the out path adds the
              # dependency via string context, without treating the text marker as a buildInput/setup-hook).
              ${pkgs.lib.concatMapStringsSep "\n"
                (d: ''cat ${d} > /dev/null'')
                (builtins.attrValues harnessRunChecks)}
              echo "ok: all harness runs passed (${toString (builtins.attrNames harnessRunChecks)})" > "$out"
            '';

            # LOCAL-GATE bindings (v-nix+v-fleet-tooling 2026-08-06, GHA-outage fallback). The 3 required
            # checks that were inline `cargoWorkspaceCheck {…}` at their attr get `let`-bound here so BOTH
            # the individual `checks.*` attrs AND the `localGate` aggregate share ONE derivation each (no
            # rebuild). Definitions are byte-identical to the former inline ones.
            fmtCheck = cargoWorkspaceCheck {
              name = "cargo-fmt";
              cargoCmd = "cargo fmt --all --check";
            };
            testCheck = cargoWorkspaceCheck {
              name = "cargo-test";
              cargoCmd = "cargo test --workspace --locked";
              src = seedTestSrc;
              extraInputs = [ pkgs.git ];
            };
            roundtripCheck = cargoWorkspaceCheck {
              name = "cargo-xtask-roundtrip";
              cargoCmd = "cargo run --locked --package xtask --profile release -- roundtrip";
              src = seedRoundtripSrc;
            };
            # emoji-lint (v-nix 2026-08-09): the GHA `emoji-lint` job (checks.yml — `cargo xtask lint-emoji`,
            # #2579) had NO nix equivalent, so under the GHA-off cutover (localGate is the sole merge gate)
            # the operator's standing NO-emoji-in-source-comments directive stopped being enforced at merge.
            # This folds it in. `emoji_free_lint` (xtask/src/main.rs) walks `implementation/**/*.rs` comments;
            # seedSrc's `implementation/seed/crates` is EXACT coverage parity (verified: all 326 .rs under
            # implementation/ live under seed/crates, 0 outside). Advisory (like the other native checks) — a
            # comment-emoji red must not be a merge-blocker beyond what the operator directive implies, but it
            # surfaces the violation. Cached xtask compile via the shared vendor, same as roundtrip.
            emojiLintCheck = cargoWorkspaceCheck {
              name = "cargo-xtask-lint-emoji";
              cargoCmd = "cargo run --locked --package xtask --profile release -- lint-emoji";
              # src = seedSrc (default): implementation/seed/crates + xtask — matches the lint's walk exactly.
            };
            # mandate-lint (v-nix 2026-08-09, v-ft/operator request): `cargo xtask lint-mandates` — the
            # mechanizable-mandate lint (currently the no-integration-tests mandate: a NEW tests/*.rs not on
            # xtask/mandate-integration-test-allowlist.txt is a DENY). Unlike emojiLintCheck (advisory), this
            # is folded INTO the localGate fail-set below so a mandate violation REJECTS the merge path
            # (operator: mandates enforced at commit-time). Same shape as emojiLintCheck: scans
            # implementation/**/*.rs + reads the allowlist; seedSrc (default) covers both (implementation/seed
            # /crates + the whole ./xtask dir, which holds the allowlist .txt). Fast native source-scan
            # (seconds, ~no gate-time add). v-ft pre-fixed a vendored-file false-positive so it won't red on
            # fold-in.
            mandateLintCheck = cargoWorkspaceCheck {
              name = "cargo-xtask-lint-mandates";
              cargoCmd = "cargo run --locked --package xtask --profile release -- lint-mandates";
              # src = seedSrc (default): implementation/seed/crates + xtask (incl. the allowlist .txt).
            };

            # LOCAL GATE — the GHA-outage fallback (operator-greenlit, concierge-assigned, v-ft leads the
            # pr-sync wiring). One `nix build .#checks.aarch64-linux.local-gate` = a single green/red over
            # EXACTLY the 9 merge-required contexts (ruleset-10 MINUS test-macos, which is native x86/macos
            # and out of scope per operator: nothing is arch-specific, aarch64 coverage is the accepted
            # fallback).
            # MACOS RESIDUAL (GHA→local-nix cutover, operator ruling 2026-08-08): with GHA dropped and this
            # local nix gate made the PRIMARY merge gate (push direct/in-batches), the test(macos-latest) leg
            # has NO nix equivalent — nix builds Linux derivations, so the macOS platform leg cannot run in the
            # aarch64-linux sandbox. The operator ACCEPTED this gap explicitly ("I do not care about macOS right
            # now, prioritize dev speed and throughput"): the test LOGIC is covered by testCheck (the same
            # cargo test, on linux), only macOS-platform-specific behavior is uncovered, and macOS has never
            # been the sole catcher of a real bug in this fleet (regressions surface on linux/wasm, the primary
            # backends). ESCAPE HATCH if a macOS-specific bug ever surfaces or dev pace slows: re-add a nightly
            # nix-on-macOS build or a minimal macos-only CI job just for that leg. Documented as the known
            # residual so the cutover's coverage story is explicit.
            # The required 9, mapped to their nix checks:
            #   rustfmt→fmtCheck · clippy→clippyShardA + clippyShardB (the 2-way shard; union == the old
            #     clippyCraneAggregate set exactly, so coverage is identical + it tracks the post-flip required
            #     contexts checks/clippy-shard-a + checks/clippy-shard-b) · test(ubuntu)→testCheck ·
            #   codegen→codegenCheck · gate→gateCheck · wasm-runtime-build→runtimeHashParity (builds the
            #   runtime component + verifies REQUIRED_RUNTIME_HASH — a superset of the raw CI build) ·
            #   syntax-roundtrip→roundtripCheck · allocation-bench→benchCheck · guide-examples→guideExamplesCheck.
            # The ADVISORY natives (cad-tests) are NOT in ruleset-10, so they
            # are deliberately EXCLUDED from the aggregate's fail-set (a red on them must not block merge,
            # matching prod). They stay independently buildable + warm via their own `checks.*` attrs; pr-sync
            # can build them separately for extra signal without gating. FAIL-CLOSED: the aggregate depends on
            # all 9 required, so `nix build` of it is red if ANY required check fails — no silent gap. aarch64.
            localGate = pkgs.runCommand "local-gate"
              {
                # The 9 merge-required-minus-macos contexts.
                # testCraneAggregate REPLACES the whole-workspace testCheck (operator 1-min-gate mandate,
                # 2026-08-09): same coverage (per-crate tests + cdz == workspace, asserted by
                # testCrateCoverageAssert), but a 1-crate edit reruns only that crate's test derivation
                # instead of the whole workspace. localGate stays the same green/red aggregate for pr-sync.
                # mandateLintCheck folded into the fail-set (v-ft/operator 2026-08-09): a mechanizable-mandate
                # DENY (e.g. a new non-allowlisted tests/*.rs) now REJECTS the merge path. It's a cheap native
                # source-scan (seconds), so it adds ~no gate time. Distinct from emojiLintCheck, which stays
                # advisory (exposed as a check but NOT in this fail-set).
                inherit clippyShardA clippyShardB codegenCheck gateCheck gateCheckRust guideExamplesCheck
                  benchCheck runtimeHashParity fmtCheck testCraneAggregate roundtripCheck
                  mandateLintCheck;
                # gateCheckRust folded into the fail-set (v-nix+v-ft 2026-08-10): closes the RUST-backend gate
                # hole — gateCheck is wasm-only, so a rust-only emit divergence (v-effects E0425 mutual-rec)
                # reached trunk green. Narrow `--case mutual` subset (rustc-per-case → full 6686 is prohibitive
                # per-MR); nightly runs the full rust gate. See gateCheckRust's def note.
                # cad-test-compiler-ml folded into the fail-set (v-ft/v-cml/concierge 2026-08-10, HARD gate):
                # it runs the compiler-ml pfq SPINE (compiler-ml Project.cdz tests = src/*.cdz incl
                # db-query-perfield.cdz), which a Core-shape edit can break — the hole broke twice. ZERO added
                # gate cost by construction: seedCompilerClosure = crateClosure cdz ++ cdz-run and rcdzc IS in
                # cdz's closure, so a Core-shape edit (rcdzc/src/core.rs, ty.rs, lower.rs) rotates seedCompilerSrc
                # → rebuilds seedCompiler → RERUNS this spine; a corpus-only / non-closure MR leaves seedCompiler
                # cached → this stays cached (cache-hit). So the nix dep graph gives the "gate Core-touching MRs
                # on the spine" conditional FOR FREE — no pr-sync git-diff logic. (bare-identifier inherit can't
                # take the hyphenated attr, so bind it explicitly from cdzCadProjectTests.)
                cadTestCompilerMl = cdzCadProjectTests.cad-test-compiler-ml;
              } ''
              echo "ok: local-gate — 9 merge-required contexts (ruleset-10 minus test-macos) + mandate-lint + cad-test-compiler-ml (Core-shape spine guard), green on aarch64-nix" > $out
            '';
          in
          {
            # the reproducibility-backstop subset the advisory `nix-flake` CI job runs (INSTEAD of a whole
            # `nix flake check`) — see the flakeReproBackstop note above. Individual attrs kept too (so
            # `nix flake check` locally still runs them + they stay independently buildable).
            flake-repro-backstop = flakeReproBackstop;
            runtime-hash-parity = runtimeHashParity;
            runtime-debug-hash-parity = runtimeDebugHashParity;
            nfc-hash-parity = nfcHashParity;
            # `nix build .#checks.<sys>.contract-hashes-valid` — the contract name→hash mapping is well-formed
            # (also part of flake-repro-backstop). The harness runs that name a contract exercise it in anger.
            contract-hashes-valid = contractHashesValid;
            # The integration-test harness runs (§9): `harness-runs` is the aggregate; each individual run is
            # exposed below as `checks.<sys>.harness-<name>` (spread from harnessRunChecks) so `nix flake
            # check` runs them all AND CI can build/cache one run in isolation. `.#packages.cdz-platform-itest`
            # is the shared, built-once integration-test executable. See the mkHarnessRun framework above.
            harness-runs = harnessRunsAll;
            # The corpus per-case caching pipeline (design/DESIGN-corpus-nix-per-case-caching.md): EVERY
            # corpus file, each case flowing through three separately-cached derivations — shred → build
            # (content-addressed `cdz-compile`) → exec (compiler-free `cdz-run --grade`). `corpus` is the
            # whole-corpus aggregate; the per-file `corpus-<file>` aggregates are spread in below.
            corpus = corpusAll;
            # The RUST-target twin of `corpus` (design gap #6): the same per-case shred → a rust build
            # (`cdz-compile -t rust`) → a rust exec (`cdz-rust-run --grade`, which compiles the emitted `.rs`'s
            # driver with `rustc` linking the pre-built `rustRlibs` + grades). `corpus-rust` is the whole-corpus
            # aggregate; the per-file `corpus-rust-<file>` aggregates are spread in below.
            corpus-rust = corpusRustAll;
            # The RUST-ASYNC target's whole-corpus aggregate (the async/gas-metered rust backend) — the last
            # corpus target to move off the native in-process `xtask gate --target rust-async` into a cached
            # nix check. Per-file `corpus-rust-async-<file>` aggregates spread in below.
            corpus-rust-async = corpusRustAsyncAll;
            # The CADENZA round-trip value-equivalence target: `corpus` with a cadenza hop
            # (program.ast → cadenza → wasm), graded vs the SAME wasm baseline so a value-miscompile in the
            # round-trip shows as a grade divergence. Per-file `corpus-cadenza-<file>` aggregates spread below.
            corpus-cadenza = corpusCadenzaAll;
            # The GLOBAL half of gap #7: a baseline case with no corpus case (silently dropped, its verdict
            # unenforced) — what the per-case `--baseline` regression check cannot see. Backend-independent.
            corpus-vanished = corpusVanishedCheck;
            # The wasm-opt OPTIMALITY-GAP sweep (advisory, never a gate constituent): the whole-corpus
            # `wasm-opt-gaps.sexp` aggregate; the per-file `wasm-opt-gaps-<file>` aggregates are spread in below
            # so a slice (e.g. 01-literals + 10-bytes) builds in isolation. See DESIGN-wasm-opt-gap-analysis-rcdzc.md.
            wasm-opt-gaps = optGapAll;
            # S3: the example project's @tests run through nix — a cache HIT when its sources are
            # unchanged (the "skip tests that haven't changed" win), a re-run + fail on a red test.
            example-project-tests = exampleProjectTests;

            # oracle-lean (L0.1): build the Lean differential oracle + run its self-test (a smoke
            # request round-trips through the frame codec + the declining handler → one Unsupported
            # verdict). Standalone/advisory — NOT in the required local-gate set; `nix flake check`
            # runs it and CI can build `.#checks.<sys>.oracle-lean-smoke` in isolation.
            oracle-lean-smoke = oracleLeanSmoke;
            # oracle-lean (L0.2): the binary-AST codec is byte-identical on real corpus-derived
            # program.ast blobs (decode → re-encode == input). Standalone/advisory, same as the smoke
            # check.
            oracle-lean-ast-roundtrip = oracleLeanAstRoundtrip;
            # L1.2 corpus conformance: the oracle asserts each corpus case; 0 mismatches required.
            oracle-lean-check = oracleLeanCheck;

            # Full-CI-in-nix increment 1: the LINT pair, mirroring checks.yml `fmt` + `clippy` exactly.
            # `nix flake check` now runs them; the checks.yml jobs stay in place (advisory overlap) until
            # v-fleet-tooling's required-set cutover retires the hand-wired ones.
            fmt = fmtCheck;
            # seq-126 Part B (option A + 1a): the whole-workspace `clippy --workspace` + `test --workspace`
            # are REPLACED by PER-CRATE checks (clippy -p C + test -p C), each shipping all member src (the
            # cargo workspace-parse floor) + ONLY its own tests/, so an independent crate's edit doesn't
            # cross-trigger and a crate's test-dir edit re-runs only its check. `fmt --all` stays
            # whole-workspace (below). closure-assert guards the fromTOML walk. Coverage parity: every
            # workspace test binary maps to one member crate (`cargo test --workspace --no-run`) → ∪ per-crate
            # == workspace; the store-dependent cdz tests self-skip with no store (same as the old
            # `cargo test --workspace`, which ran storeless). extraSrc: spec/semantics (corpus round-trip),
            # compiler-ml (cdz run_ml driver), cdz-runtime/src/bigint.rs (cdz-num `#[path]`-includes it →
            # every crate whose closure has cdz-num: cdz-num/cdz-calc/rcdzc). extraInputs: git (xtask fleet
            # batch tests).
            crate-closure-assert = crateClosureAssert;
            # cdz = WORKSPACE-SRC (concierge-confirmed 1a), NOT closure/tests-dir-scoped like the other 10.
            # WHY cdz differs: its run_rust_cli tests are WORKSPACE-INTEGRATION — they rustc-compile emitted
            # Rust linking the sibling cdz-num/cdz-rt rlibs "beside the cdz bin", which only a full-workspace
            # build lays out (a bare `-p cdz` does NOT even emit libcdz_num.rlib — cdz uses cdz-num only
            # transitively via rcdzc → E0433). So crateCdzCheck does `cargo build --workspace` FIRST (lays
            # every rlib) THEN `test -p cdz`, from seedTestSrc (crates+xtask+compiler-ml+spec/semantics; git
            # for xtask fleet batch tests). cdz is the TOP crate (reruns on ~every edit anyway → tests-dir
            # granularity is ~nil), so workspace-src costs ~nothing AND is DRIFT-FREE — a `--test`-exclusion
            # list to keep it closure-scoped would silently drop a new cdz test (the coverage regression the
            # parity guard forbids). Do NOT "fix" this back to a split.
            crate-cdz = crateCdzCheck;
            # The REQUIRED clippy contexts = the 2-WAY SHARD (see the clippyShardA/B bindings): per-crate crane
            # cargoClippy (shared cargoArtifacts → deps compiled ONCE), split into 2 parallel jobs to halve the
            # ~8.8m clippy critical-path pole. The former single `clippy` attr (= clippyCraneAggregate, the whole
            # set) was RETIRED as a checks.yml/ruleset context 2026-08-07 (STEP 2/3); clippyCraneAggregate stays
            # as a `let` binding (union-parity reference + independently buildable via `nix flake check`), just
            # no longer a required `checks.*` attr. Union == clippyCraneAggregate's set exactly (5+6=11).
            clippy-shard-a = clippyShardA;
            clippy-shard-b = clippyShardB;
            # `checks.test` = a whole-workspace `cargo test --workspace --locked` (NOT crane). Option-b (v-ft
            # crane re-measure, 2026-08-05): crane cargoTest NARROWED but did not erase a test-ubuntu regression
            # — it settled ~18-19m warm vs the ~16m cargo baseline, because per-crate cargoTest recompiles
            # proc-macro dev-deps (serde_with/json/derive) that the deps-only cargoArtifacts warm-cache can't
            # share (feature-unification/fingerprint diffs between the deps-only build and `-p C`). clippy KEEPS
            # its crane win (16→8m, cargoClippy shares the cache cleanly); test reverts to the cargo path so it's
            # back at its ~16m baseline (neutral) instead of the crane regression, and drops the cargoTest
            # complexity. Coverage parity = the INC 2 whole-workspace run (`cargo test --workspace` = ∪ all
            # member test binaries; store-dependent cdz tests self-skip storeless, same as pre-crane). extraSrc
            # via seedTestSrc (crates+xtask+compiler-ml+spec/semantics); git for xtask fleet batch tests. If test
            # <16m is wanted later, SHARD the cargo test job (parallel -p groups) — attacks the first-party
            # compile floor without the crane dev-dep-recompile penalty (v-ft's sharding lever). Context name
            # unchanged (`checks / test (ubuntu-latest)`) → no ruleset edit.
            test = testCheck;
            # PER-CRATE TEST aggregate (operator 1-min-gate mandate, 2026-08-09): the per-crate-incremental
            # replacement for the whole-workspace `test` that localGate now folds. Same coverage (asserted by
            # test-crate-coverage-assert), but a 1-crate edit reruns only that crate's test derivation. The
            # whole-workspace `test` above is KEPT (standalone / the GHA `test (ubuntu-latest)` context name).
            test-crane-aggregate = testCraneAggregate;
            test-crate-coverage-assert = testCrateCoverageAssert;
            # Full-CI-in-nix increment 3: the native half of the GHA rcdzc-wasm job (the wasm build half
            # is the rcdzcWasm derivation / rcdzc-wasm-hash, already covered).
            rcdzc-wasm-native = rcdzcWasmNativeCheck;
            # Full-CI-in-nix increment 6b: the GHA codegen job (cargo xtask codegen --check, ABI staleness).
            codegen-check = codegenCheck;
            # Full-CI-in-nix increment 6c: the GHA gate job (cargo xtask gate --check — THE behavior gate).
            gate-check = gateCheck;
            gate-check-rust = gateCheckRust;
            # Full-CI-in-nix increment 6d: the GHA bench job (cargo xtask bench — runtime alloc ceilings).
            bench-check = benchCheck;
            # Full-CI-in-nix increment 6e: the GHA cad-tests job (cdz test on the 4 in-tree Cadenza projects).
            # PER-PROJECT SPLIT (2026-08-08): `cad-tests` is now an aggregate over the 4 per-project
            # derivations (each with its own narrow fileset — a one-project change reruns only that one).
            # The 4 are ALSO exposed individually so a candidate touching one project builds just it.
            cad-tests = cdzCadTestsCheck;
            # Full-CI-in-nix increment 6f: the GHA guide-examples job (the guide's runnable-content gate —
            # hermetic wasm-pack + npm ci + the check:* battery + build + bundle). The LAST required job.
            guide-examples = guideExamplesCheck;
            # Full-CI-in-nix increment 6a: the GHA `roundtrip` job — every corpus program round-trips
            # through the syntax surfaces. Corpus-only (reads spec/semantics, no runtime store) → narrow
            # `seedRoundtripSrc` (no compiler-ml, #2007). Invoked via `cargo run --locked` (not the bare
            # `cargo xtask` alias, which omits --locked) so a lockfile drift hard-fails, matching the
            # workspace test/clippy checks (#2032).
            roundtrip = roundtripCheck;
            # emoji-lint: the GHA emoji-lint job's nix equivalent (cargo xtask lint-emoji, comment-scoped
            # NO-emoji ban over implementation/**/*.rs). Folded in since GHA-off made localGate the sole gate.
            emoji-lint = emojiLintCheck;
            # mandate-lint: cargo xtask lint-mandates (no-integration-tests + future mechanizable mandates).
            # Folded into localGate's FAIL-SET (above) so a violation blocks the merge path (operator).
            mandate-lint = mandateLintCheck;
            # LOCAL GATE aggregate — the GHA-outage fallback (see the `localGate` binding above). pr-sync
            # invokes `nix build .#checks.aarch64-linux.local-gate` for a single green/red over the 9
            # merge-required contexts (ruleset-10 minus test-macos) without any GH runner.
            local-gate = localGate;
          }
          # seq-126 Part B: expose each per-crate CRANE CLIPPY check individually (granular signal + `nix flake
          # check` runs them). checks.clippy forces this same set; exposing them adds per-crate cache
          # granularity + a precise red when one crate fails. These are cargoArtifacts-cached.
          // perCrateClippyCrane
          # PER-CRATE TEST checks (operator 1-min-gate mandate, 2026-08-09): expose the per-crate crane
          # cargoTest derivations individually (checks.<sys>.test-<crate>) alongside the test-crane-aggregate.
          # A candidate touching ONE crate builds just its test-<crate> (+ dependents); the rest cache-hit.
          # cargoArtifacts-cached (deps + dev-dep layer warm since cargoArtifacts is doCheck=true).
          // perCrateTestCrane
          # PER-PROJECT cad-tests split (2026-08-08): expose the 4 per-project `cdz test` derivations
          # individually (checks.<sys>.cad-test-{cad,compiler-ml,choreography,iterators}) alongside the
          # `cad-tests` aggregate. A candidate touching ONE project builds just that project's check; the
          # aggregate (required context) still forces all 4. cache-warm roots these the same as the aggregate.
          // cdzCadProjectTests
          # PER-RUN integration-test harness checks (operator #2994 review): expose each harness run
          # individually as `checks.<sys>.harness-<name>` alongside the `harness-runs` aggregate, so a
          # candidate touching ONE run's spec (or one program) rebuilds just the affected run(s) — the
          # binary + untouched programs + other runs all cache-hit. Auto-discovered from the harness-runs dir.
          // (pkgs.lib.mapAttrs' (n: v: pkgs.lib.nameValuePair "harness-${n}" v) harnessRunChecks)
          # PER-FILE corpus aggregates: `corpus-<file>` for every corpus file, so CI can build/cache one
          # file's whole per-case graph in isolation (the top-level `corpus` forces them all). The per-CASE
          # derivations (`corpus-build/exec-<file>-<idx>`) are cached transitively through these — a candidate
          # touching one case rebuilds just that case's chain, and a compiler change that re-emits identical
          # wasm cache-hits every exec (content-addressed build + store).
          // corpusFileAggs
          # PER-FILE rust corpus aggregates: `corpus-rust-<file>` for every corpus file (the rust-target twin
          # of the wasm `corpus-<file>` set), so CI can build/cache one file's rust per-case graph in isolation
          # (the top-level `corpus-rust` forces them all).
          // corpusRustFileAggs
          # PER-FILE rust-ASYNC corpus aggregates: `corpus-rust-async-<file>` (the async rust-target twin),
          # so CI can build/cache one file's async per-case graph in isolation (top-level `corpus-rust-async`
          # forces them all). Moves the last native `xtask gate --target rust-async` path into cached nix.
          // corpusRustAsyncFileAggs
          # PER-FILE cadenza round-trip aggregates: `corpus-cadenza-<file>` (compile→cadenza→wasm, graded vs
          # the wasm baseline), so one file's cadenza per-case graph builds/caches in isolation (top-level
          # `corpus-cadenza` forces them all).
          // corpusCadenzaFileAggs
          # PER-FILE wasm-opt-gap aggregates: `wasm-opt-gaps-<file>` for every corpus file, so a slice
          # (01-literals + 10-bytes) builds in isolation while the top-level `wasm-opt-gaps` forces the whole
          # sweep. Per-CASE reports are CA on {emit, binaryen} → shared with `wasm-opt-gaps` + cached.
          // optGapFileAggs;

        devShells.default = pkgs.mkShell {
          # THE SINGLE dev shell everyone runs everything in (operator 2026-08-28): one uniform
          # environment, no per-lane shells. All EXTERNAL/SUBSTITUTABLE tooling (fetched from the binary
          # cache, shared ONCE per box via /nix/store — not a per-agent compile), so eager is fine:
          #   rustToolchain : rustc/cargo/clippy/rustfmt/rust-src + wasm32 target (from the pin)
          #   wasm-tools    : the runtime component build + `cdz test` need it (nixpkgs pin)
          #   cargo-component : the runtime component build is `cargo component build` — pinned 0.21.1
          #                   (the version the recorded REQUIRED_RUNTIME_HASH was produced with), from
          #                   nixpkgs not the host ~/.cargo/bin (hermeticity)
          #   lean4         : lean + lake for the differential oracle (implementation/oracle-lean/). Folded
          #                   in from the retired `devShells.oracle` — a substitutable ~2.5GiB fetch shared
          #                   once per box, so the single-shell simplicity is worth it (supersedes the
          #                   2026-08-27 closure-hygiene split; operator wants ONE shell). `.#oracle` is
          #                   kept below as a deprecated ALIAS to this shell so no caller breaks.
          # LOCAL builds are NOT eager here — the shellHook defers them (see the lazy-boot note below).
          packages = [
            rustToolchain
            pkgs.wasm-tools
            pkgs.cargo-component
            pkgs.lean4
          ];

          # R4: point cdz/cdz-run at the NIX-BUILT component store. cdz-run + cdz `default_store()`
          # resolve `CDZ_STORE` (env) before the compiled `target/cadenza-store` fallback (the --store
          # flag still wins over the env); the content-address re-hash-verify on load is untouched, so a
          # wrong store entry is caught, not silently loaded. So exporting CDZ_STORE=<packages.store>
          # makes `cdz run`/`cdz test` inside `nix develop` resolve every component (runtime + NFC +
          # guests) from the nix-built, content-addressed store — the operator's load-by-hash north star.
          # OPT-IN + non-destructive: `cargo xtask build` (the store WRITER) still writes
          # target/cadenza-store; this only overrides the READ path for a nix-develop session.
          # LAZY BOOT (operator 2026-08-28): agents boot directly into this shell, so it must be REACTIVE.
          # A LOCALLY-BUILT derivation referenced in the shellHook is realised at BOOT (nix must build it to
          # substitute its path) — previously `CDZ_STORE=<the component store>` forced 9 local derivations
          # (runtime component + debug + NFC + guests + hashes, minutes cold) on every fresh `nix develop`.
          # Rule: EAGER for external/substitutable tooling (rustToolchain/wasm-tools/cargo-component — the
          # `packages` above, fetched from the binary cache), LAZY for anything derived from LOCAL builds
          # (the component store, the compiler) — deferred to first actual use. So the shellHook references
          # NO local derivation; CDZ_STORE is resolved on the first cdz/cdz-run call and memoized.
          shellHook = ''
            export NIX_REMOTE=daemon
            export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-8}"
            # Resolve the flake root INSIDE each call (not a shell var) so the functions work from any
            # subdir and after `export -f` into bash children/scripts (an unexported var would be lost there).
            __cdz_flakeroot() { git rev-parse --show-toplevel 2>/dev/null || echo "$PWD"; }
            # LAZY store: build + pin the nix component store (runtime/NFC/guests) on the FIRST cdz/cdz-run
            # use (memoized for the session), so `cdz run`/`cdz test` resolve components by hash from the nix
            # store rather than a `target/cadenza-store` fallback — WITHOUT paying that build at boot.
            __cdz_ensure_store() {
              if [ -z "''${CDZ_STORE:-}" ]; then
                CDZ_STORE="$(nix build --no-link --print-out-paths --option warn-dirty false "$(__cdz_flakeroot)#store")" && export CDZ_STORE
              fi
            }
            # ALL-NIX AGENT ENTRYPOINTS: invoke the tool directly — nix compiles it ON DEMAND from your
            # CURRENT worktree (picks up uncommitted edits to TRACKED files; new untracked files need
            # `git add`) reusing the warm dep-closure, so there is no bare-cargo per-worktree cold rebuild.
            # FUNCTIONS not PATH: `nix run` rebuilds from the dirty tree each call, whereas a PATH-injected
            # binary would freeze at shell-entry rev and miss your edits (v-nix+operator 2026-08-28).
            cdz()         { __cdz_ensure_store; nix run --option warn-dirty false "$(__cdz_flakeroot)#cdz"         -- "$@"; }
            cdz-run()     { __cdz_ensure_store; nix run --option warn-dirty false "$(__cdz_flakeroot)#cdz-run"     -- "$@"; }
            cdz-compile() { nix run --option warn-dirty false "$(__cdz_flakeroot)#cdz-compile" -- "$@"; }
            gate()        { nix run --option warn-dirty false "$(__cdz_flakeroot)#gate"        -- "$@"; }
            fast-gate()   { nix run --option warn-dirty false "$(__cdz_flakeroot)#fast-gate"   -- "$@"; }
            # cdz-help — print every custom shell command available here, on demand (also shown at boot).
            # Namespaced `cdz-help` (NOT `help`, which is a bash builtin) so agents can re-inspect anytime.
            cdz-help() {
              echo "cdz all-nix shell — custom commands (nix compiles on demand from your worktree,"
              echo "reusing the warm cache; run 'cdz-help' anytime to reprint this):"
              echo "  cdz …               compile / run / test / doctor  (builds the component store on 1st run)"
              echo "  cdz-run FILE.wasm   run a component"
              echo "  cdz-compile …       the standalone compiler (what cdz delegates to)"
              echo "  fast-gate [crates]  fast touched-crate gate (inner loop)"
              echo "  gate                full local-gate battery (convenience)"
              echo "  cdz-help            print this list"
              echo "  → authoritative MERGE gate stays: cargo xtask fleet gate-local"
            }
            export -f __cdz_flakeroot __cdz_ensure_store cdz cdz-run cdz-compile gate fast-gate cdz-help 2>/dev/null || true
            cdz-help
          '';
        };

        # DEPRECATED ALIAS: `.#oracle` == the single `default` shell (operator 2026-08-28: one shell for
        # everyone). lean4 folded into `default`, so there's no separate oracle environment anymore; this
        # alias only keeps `nix develop .#oracle` working for callers (window.sh / scripts) until they
        # migrate to plain `nix develop`. Remove once nothing references `.#oracle`.
        devShells.oracle = self.devShells.${system}.default;

        # ── LOCAL WARM-KEEP (v-nix+v-fleet-tooling 2026-08-08) ─────────────────────────────────────
        #
        # `nix run .#warm-keep` — keeps the LOCAL /nix/store hot for local-gating. Background: the
        # operator ran out of GHA credits (2026-08-08), so the fleet gates every MR LOCALLY via
        # `cargo xtask gate-local` / `.#checks.<sys>.local-gate`. cache-warm.yml (which kept the GHA
        # cache hot) is itself a GHA workflow — DEAD now — so the local host's /nix/store is the ONLY
        # warm source. If the heavy warm layer (crane's ~341M cargo-artifacts dep-closure + the
        # component store) gets garbage-collected between MRs, gate-local drops from ~1s/warm (or ~7-8m
        # for the changed-crate tier) to a full COLD rebuild — prohibitive per-MR. This app REALISES +
        # pins that warm layer as durable GC-roots (via `--out-link`, which registers an indirect GC
        # root), so `nix-collect-garbage` never reclaims it. v-ft's gate-local/drain invokes this (or a
        # host timer runs it periodically) to replace the dead cache-warm.yml with a LOCAL warm-keep.
        # Idempotent + fast when already warm (all cache-hit); pins the SAME set the local gate builds.
        apps.warm-keep =
          let
            warmKeep = pkgs.writeShellApplication {
              name = "cdz-warm-keep";
              runtimeInputs = [ pkgs.nix pkgs.coreutils ];
              text = ''
                # GC-root dir: default to an ABSOLUTE per-user path OUTSIDE any git worktree. A --out-link
                # indirect GC-root is registered at the link's path; if that path is repo-relative and the
                # worktree is later cleaned/moved/removed, the link dangles and nix DROPS the root, silently
                # unrooting the warm layer right before a GC (an observed near-miss). An absolute per-user
                # dir survives worktree churn. Override with CDZ_WARM_ROOT (the host GC runner sets it).
                root_dir="''${CDZ_WARM_ROOT:-$HOME/.cdz-warm-roots}"
                mkdir -p "$root_dir"
                echo "cdz warm-keep: pinning the local warm layer as GC-roots under $root_dir/ (system ${system})"
                # The heavy layers gate-local depends on, each rooted so the pre-save GC keeps them hot:
                #  - the crane dep-closure (~341M) at BOTH profiles: dev (cargo-artifacts, clippy/test) +
                #    release (cargo-artifacts-release + -release-codegen, for gate/codegen/bench — build-inputs
                #    that go dead after those checks build, so without their own root a corpus MR rebuilds the
                #    release deps cold, negating the crane conversion);
                #  - the component store + the local-gate aggregate (pulls the 9 required checks' closure);
                #  - the CORPUS aggregates (wasm / rust / rust-async / cadenza) + the wasm-opt-gaps sweep:
                #    their per-case CA build+exec outputs were UNROOTED, so the store GC evicted them, forcing
                #    agents into COLD whole-corpus FROM-SOURCE sweeps — which overloaded the daemon (builders
                #    died → stale build-locks → the fleet-wide daemon WEDGE, ~15 clients stuck 9-12h, 2026-08-28).
                #    Rooting the aggregates keeps every per-case CA output hot so a corpus check cache-HITS
                #    instead of rebuilding from source. This is the durable fix for the recurring wedge.
                # SHARDED PER-TARGET (v-nix 2026-08-28): each target gets its OWN --out-link and builds
                # SEQUENTIALLY, so its GC-root is registered the MOMENT that target finishes. The prior
                # monolithic `nix build A B C … --out-link warm` wrote NO symlink until the WHOLE set
                # completed — so if any single member never converged (a corpus aggregate racing a
                # fast-moving main whose per-case CA hashes churn, or the heavy wasm-opt-gaps sweep),
                # ZERO roots appeared and the corpus stayed cold (observed ALL-session 2026-08-28 by
                # v-ft: only 8 stale deps/store roots, zero corpus-*; 10 clients still cold-sweeping).
                # Sharding lets fast/foundational targets root FIRST and a slow/churning one starve only
                # itself. Sequential (one build client at a time) also caps daemon pressure — no
                # self-inflicted contention. Best-effort: a failed shard logs WARN and we continue (a
                # supervisor re-run cache-HITs the rooted ones and retries the rest); we exit 0 so a
                # cron/nohup supervisor doesn't alarm on a partial pass.
                # First drop the OLD monolithic-era roots (warm, warm-<N>): they pin STALE closures the
                # current gates don't want, so removing them lets the next GC reclaim that churn; the
                # loop below immediately re-establishes fresh per-target roots (warm-<slug>).
                # rm -f removes the symlink itself (live OR dangling target) and ignores the literal
                # glob when nothing matches (-f), so no set -e trip and stale dangling roots are cleared.
                rm -f "$root_dir"/warm "$root_dir"/warm-[0-9]*
                # --out-link registers each as an indirect GC-root so the store stays hot.
                targets=(
                  "packages.${system}.cargo-artifacts"
                  "packages.${system}.cargo-artifacts-release"
                  "packages.${system}.cargo-artifacts-release-codegen"
                  "packages.${system}.store"
                  "checks.${system}.local-gate"
                  "checks.${system}.corpus"
                  "checks.${system}.corpus-rust"
                  "checks.${system}.corpus-rust-async"
                  "checks.${system}.corpus-cadenza"
                  "checks.${system}.wasm-opt-gaps"
                )
                for t in "''${targets[@]}"; do
                  slug="''${t##*.}"
                  echo "cdz warm-keep: rooting .#$t → $root_dir/warm-$slug"
                  if nix build ".#$t" --out-link "$root_dir/warm-$slug" --print-build-logs; then
                    echo "cdz warm-keep: rooted $slug"
                  else
                    echo "cdz warm-keep: WARN could not root $slug (continuing)"
                  fi
                done
                echo "cdz warm-keep: done — sharded warm layer pinned (per-target roots under $root_dir/warm-*)."
              '';
            };
          in
          {
            type = "app";
            program = "${warmKeep}/bin/cdz-warm-keep";
          };

        # ── LOCAL STORE GC (v-nix+v-fleet-tooling 2026-08-08) ──────────────────────────────────────
        #
        # `nix run .#gc` — reclaim the local /nix/store churn while PRESERVING the warm layer. The pair
        # to apps.warm-keep: warm-keep PINS the heavy layer (crane deps + component store + local-gate)
        # as GC-roots; this reclaims everything ELSE. Gating locally (whether via schedule-pass
        # --local-gate or a candidate-PR gate-local) rebuilds a fresh closure per MR, so the store grows
        # unboundedly (measured: 6198-6883 dead paths / ~31G within a day). nix store gc respects the
        # GC-roots warm-keep registered (indirect roots under CDZ_WARM_ROOT), so the warm layer is NEVER
        # reclaimed — this GCs only unrooted dead paths. Trigger-AGNOSTIC: the host runner (v-ft's lane)
        # decides WHEN; this app is just the WHAT. Two valid cadences the runner picks between depending on
        # the active integration model (which whipsawed a few times around 2026-08-08 as GHA credit status
        # was reconfirmed): (a) if pr-sync runs `schedule-pass --local-gate --execute` as its loop, hook GC
        # as the LAST step of each drain (post-drain hook) so it tracks actual churn; (b) if pr-sync is on
        # the CI-gated candidate-PR path (no local-gate drain to hook), run GC on a ~3h wall-clock timer.
        # apps.gc is needed either way — local gate-local runs + dev builds churn the store regardless of
        # model. Always run warm-keep FIRST (via the runner) so the current warm layer is freshly rooted
        # before GC.
        # CDZ_GC_MAX_FREED (bytes, optional): cap the reclaim per run via `nix store gc --max` so a single GC pass
        # is bounded (avoids a long stall on a huge backlog); unset = reclaim all dead paths.
        apps.gc =
          let
            gc = pkgs.writeShellApplication {
              name = "cdz-gc";
              runtimeInputs = [ pkgs.nix pkgs.coreutils ];
              text = ''
                echo "cdz gc: reclaiming dead /nix/store paths (warm-keep GC-roots are preserved)"
                # Show what is rooted so an operator can confirm the warm layer is protected before GC.
                echo "cdz gc: live GC-roots referencing the warm layer:"
                nix-store --gc --print-roots 2>/dev/null | grep -iE "warm|local-gate|component-store|seed-deps" || true
                if [ -n "''${CDZ_GC_MAX_FREED:-}" ]; then
                  echo "cdz gc: bounded pass — reclaiming up to $CDZ_GC_MAX_FREED bytes"
                  # `nix store gc` bounds the reclaim with `--max <bytes>` (Stop after freeing n bytes).
                  # NOT `--max-freed` — that was the OLD `nix-collect-garbage` flag; the `nix store gc`
                  # subcommand renamed it to `--max`, and `--max-freed` errors "unrecognised flag" on this
                  # nix (Determinate 3.21.9 / 2.34.8), so the bounded pass reclaimed NOTHING (v-ft, first
                  # live gc-hook fire 2026-08-08). CDZ_GC_MAX_FREED keeps its name (the runner's env contract).
                  nix store gc --max "$CDZ_GC_MAX_FREED"
                else
                  nix store gc
                fi
                echo "cdz gc: done — dead paths reclaimed, warm layer intact."
              '';
            };
          in
          {
            type = "app";
            program = "${gc}/bin/cdz-gc";
          };

        # apps.fast-gate — the PER-AGENT fast inner-loop dev gate (operator per-agent-latency priority
        # 2026-08-10, concierge-greenlit). PROBLEM: an agent running the FULL localGate battery (~8-15min)
        # before every MR pays the whole fleet-wide gate cost on each iteration, which dominates its cycle.
        # FIX: this runs ONLY the touched crate's per-crate checks (test-<crate> + clippy-<crate>, or
        # crate-cdz for cdz), warm-cached = seconds-to-~2min, giving fast feedback WITHOUT the full battery.
        # It auto-detects touched crates from `git diff --name-only` (vs origin/main by default, override
        # with an explicit arg list of crate names). fmt is whole-tree + cheap so it always runs.
        #
        # NOT A MERGE GATE: this is NARROWER by design — it does NOT run the integration checks (guide,
        # codegen, bench, gate, native, hash-parity, cross-crate dependents beyond the touched crate), so a
        # green here does NOT mean merge-safe. pr-sync's FULL localGate stays the authoritative pre-merge
        # catch. The tool PRINTS that caveat on green so no agent mistakes fast-green for merge-clearance
        # (concierge requirement). Reserve the full battery for pr-sync's integration pass (unchanged).
        apps.fast-gate =
          let
            # crate → the check attr(s) that cover it. cdz is a combined `crate-cdz` (test+clippy in one);
            # every other root crate has `test-<c>` + `clippy-<c>`. Rendered to bash `case` arms below.
            crateChecks = name:
              if name == "cdz" then [ "crate-cdz" ]
              else [ "test-${name}" "clippy-${name}" ];
            # bash `case` arms mapping a crate DIR PREFIX → its space-joined check attrs (for git-diff detect).
            dirCaseArms = pkgs.lib.concatStringsSep "\n" (map
              (c: ''            ${rootWorkspaceCrates.${c}}/*) echo "${pkgs.lib.concatStringsSep " " (crateChecks c)}" ;;'')
              rootCrateNames);
            # bash `case` arms mapping an explicit crate NAME arg → its check attrs.
            nameCaseArms = pkgs.lib.concatStringsSep "\n" (map
              (c: ''            ${c}) echo "${pkgs.lib.concatStringsSep " " (crateChecks c)}" ;;'')
              rootCrateNames);
            fastGate = pkgs.writeShellApplication {
              name = "cdz-fast-gate";
              runtimeInputs = [ pkgs.nix pkgs.coreutils pkgs.git ];
              text = ''
                # Map a changed path to its crate's check attrs (empty = a path in no gated root crate).
                path_checks() {
                  case "$1" in
                ${dirCaseArms}
                    *) echo "" ;;
                  esac
                }
                # Map an explicit crate-name arg to its check attrs.
                name_checks() {
                  case "$1" in
                ${nameCaseArms}
                    *) echo "" ;;
                  esac
                }
                checks=""
                if [ "$#" -gt 0 ]; then
                  # Explicit crate-name args.
                  for c in "$@"; do
                    got="$(name_checks "$c")"
                    if [ -z "$got" ]; then echo "cdz fast-gate: '$c' is not a gated root crate — skipping" >&2; else checks="$checks $got"; fi
                  done
                else
                  # Auto-detect from git diff --name-only vs origin/main (the touched-crate set).
                  base="''${CDZ_FAST_GATE_BASE:-origin/main}"
                  echo "cdz fast-gate: detecting touched crates from git diff --name-only $base"
                  while IFS= read -r f; do
                    [ -n "$f" ] || continue
                    got="$(path_checks "$f")"
                    [ -n "$got" ] && checks="$checks $got"
                  done < <(git diff --name-only "$base" 2>/dev/null)
                fi
                # Dedup the check set.
                checks="$(echo "$checks" | tr ' ' '\n' | sort -u | grep -v '^$' | tr '\n' ' ')"
                if [ -z "$checks" ]; then
                  echo "cdz fast-gate: no touched gated crate detected — nothing to build (a non-crate edit, e.g. docs/corpus, is not covered by a per-crate check; use the full localGate for those)."
                  exit 0
                fi
                echo "cdz fast-gate: building touched-crate checks (warm-cached):$checks"
                # shellcheck disable=SC2086
                attrs=""; for c in $checks; do attrs="$attrs .#checks.${system}.$c"; done
                # fmt is whole-tree + cheap — always include it so a formatting slip is caught fast.
                attrs="$attrs .#checks.${system}.fmt"
                # shellcheck disable=SC2086
                if nix build $attrs --print-build-logs; then
                  echo ""
                  echo "cdz fast-gate: GREEN — the touched crate(s) pass test + clippy + fmt."
                  echo "⚠ NOT MERGE-SAFE: this is the NARROW inner-loop gate (touched crate only). It does NOT"
                  echo "  run integration checks (guide/codegen/bench/gate/native/hash-parity) or cross-crate"
                  echo "  dependents. pr-sync's FULL localGate is the authoritative pre-merge catch — a green"
                  echo "  here means 'fast feedback OK to keep iterating', not 'ready to land'."
                else
                  echo "cdz fast-gate: RED — a touched-crate check failed above. Fix + re-run." >&2
                  exit 1
                fi
              '';
            };
          in
          {
            type = "app";
            program = "${fastGate}/bin/cdz-fast-gate";
          };

        # apps.cdz / apps.cdz-run — run the compiler + runtime THROUGH NIX (operator all-nix mandate,
        # 2026-08-28: agents should not invoke bare `cargo`, which cold-rebuilds the dep closure per
        # worktree — ~177GB of duplicated target/ dirs, wasmtime/cranelift recompiled ~40x). Both wrap
        # the crane-built `seedCompiler` ($out/bin/{cdz,cdz-run}), so they REUSE the warm dep-closure
        # (the ~383MB cargoArtifacts layer, GC-root-pinned by warm-keep) — no per-worktree cold rebuild.
        #   nix run .#cdz -- run prog.cdz       → the unified CLI (compile / run / test / doctor)
        #   nix run .#cdz -- test               → the @test suite
        #   nix run .#cdz-run -- prog.wasm      → the standalone component runner / grader
        # The tight inner loop stays `nix run .#fast-gate`; the full merge gate is
        # `nix build .#checks.<system>.local-gate`. Together these remove every reason to reach for raw
        # cargo (v-fleet-tooling wires the boot-into-nix-develop + the cargo-redirect wrapper).
        #
        # WRAPPED (not a bare bin): the nix `seedCompiler` builds `cdz` in DELEGATE mode
        # (`--no-default-features`, v-cdz-delegate #3397) — so `cdz compile` SPAWNS the external
        # `cdz-compile` binary rather than linking rcdzc, and needs `$CDZ_COMPILE_BIN` set (else
        # `cdz: cdz-compile not found`). And `cdz run`/`cdz test` resolve the runtime/NFC/guest components
        # via `$CDZ_STORE`. So each app is a thin wrapper that injects both (respecting a caller override
        # via `:-`), exactly as the flake's corpus checks do (flake.nix ~L665/1555) — making the app
        # SELF-CONTAINED (works outside `nix develop` too). Because `nix run .#cdz` evaluates the CURRENT
        # (dirty) flake, `cdzCompile`/`componentStore` still rebuild-on-edit from the worktree.
        apps.cdz =
          let
            cdzw = pkgs.writeShellApplication {
              name = "cdz";
              runtimeInputs = [ ];
              text = ''
                export CDZ_COMPILE_BIN="''${CDZ_COMPILE_BIN:-${cdzCompile}/bin/cdz-compile}"
                export CDZ_STORE="''${CDZ_STORE:-${componentStore}}"
                exec "${seedCompiler}/bin/cdz" "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${cdzw}/bin/cdz";
          };
        apps.cdz-run =
          let
            cdzrunw = pkgs.writeShellApplication {
              name = "cdz-run";
              runtimeInputs = [ ];
              text = ''
                export CDZ_STORE="''${CDZ_STORE:-${componentStore}}"
                exec "${seedCompiler}/bin/cdz-run" "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${cdzrunw}/bin/cdz-run";
          };
        # apps.cdz-compile — the STANDALONE compiler (rcdzc's `cdz-compile` bin) directly, bypassing
        # `cdz`'s delegate spawn. Same bin `cdz compile` delegates to; useful to invoke the compiler
        # on its own (`nix run .#cdz-compile -- prog.sexp -t wasm -o out.wasm`). No CDZ_STORE (compile
        # only emits; it does not run). Rebuilds-on-edit like the others (dirty-flake eval).
        apps.cdz-compile = {
          type = "app";
          program = "${cdzCompile}/bin/cdz-compile";
        };

        # apps.gate — a uniform `nix run .#gate` convenience surface for the FULL local-gate battery
        # (all-nix mandate). CONVENIENCE ONLY: the AUTHORITATIVE merge gate stays `cargo xtask fleet
        # gate-local` (fleet.rs), which wraps this same local-gate build WITH the check-lease slot (#4997),
        # failing-sub-check naming (#4868), and transient-vs-real advisory (#4891) — none of which a bare
        # build has. Use `gate` for a quick local full-gate look; gate-local stays the pre-merge gate.
        apps.gate =
          let
            gate = pkgs.writeShellApplication {
              name = "cdz-gate";
              runtimeInputs = [ pkgs.nix ];
              text = ''
                echo "cdz gate: nix build .#checks.${system}.local-gate (full battery; CONVENIENCE — the" >&2
                echo "          authoritative merge gate is 'cargo xtask fleet gate-local')…" >&2
                exec nix build ".#checks.${system}.local-gate" --print-build-logs "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${gate}/bin/cdz-gate";
          };

        # apps.wasm-opt-gaps — run the wasm-opt optimality-gap sweep + refresh the tracked
        # `implementation/design/wasm-opt-gaps.sexp` (design/DESIGN-wasm-opt-gap-analysis-rcdzc.md). Builds the
        # `wasm-opt-gaps` check (per-case unbundle → `wasm-opt --all-features -O3`/`-Oz` → record, all cached +
        # parallel) and copies the aggregate into the tree so v-wasm-opt reads/ranks a committed doc.
        #   nix run .#wasm-opt-gaps            → refresh the default doc path + print it
        #   nix run .#wasm-opt-gaps -- PATH    → write to PATH instead
        apps.wasm-opt-gaps =
          let
            writer = pkgs.writeShellApplication {
              name = "cdz-wasm-opt-gaps";
              runtimeInputs = [ pkgs.nix pkgs.coreutils ];
              text = ''
                out="''${1:-implementation/design/wasm-opt-gaps.sexp}"
                echo "cdz wasm-opt-gaps: building the corpus-wide sweep (unbundle + wasm-opt per case; cached)…" >&2
                p=$(nix build --no-link --print-out-paths ".#checks.${system}.wasm-opt-gaps")
                cp "$p" "$out"
                echo "cdz wasm-opt-gaps: wrote $out" >&2
                cat "$out"
              '';
            };
          in
          {
            type = "app";
            program = "${writer}/bin/cdz-wasm-opt-gaps";
          };
      });
}
