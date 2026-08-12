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

  # ca-derivations is REQUIRED to evaluate/build this flake: reducer-cadenza-genesis is content-addressed
  # (perf: it stops corpus-closure-rotation from re-running the genesis E2E — see its maker note). Declaring
  # it here in nixConfig makes it AMBIENT for every flake consumer — GHA runners, a fresh dev machine, a local
  # `nix flake check` — instead of assuming it's set host-side. Fixes the advisory nix-flake-check RED
  # (`experimental Nix feature 'ca-derivations' is disabled` while evaluating reducerCadenzaGenesisValid): the
  # DeterminateSystems GHA runner had nix-command+flakes but NOT ca-derivations, and the earlier
  # comment's "enabled fleet-wide" only held for the fleet HOST (via nix.custom.conf), not GHA. `extra-`
  # prefix APPENDS (doesn't clobber the caller's nix-command/flakes). v-nix 2026-08-10, github-liaison report.
  nixConfig.extra-experimental-features = [ "ca-derivations" ];

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
          cdz-corpus = "implementation/seed/crates/cdz-corpus";
          cdz-num = "implementation/seed/crates/cdz-num";
          cdz-rt = "implementation/seed/crates/cdz-rt";
          cdz-run = "implementation/seed/crates/cdz-run";
          cdz-rust-render = "implementation/seed/crates/cdz-rust-render";
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
              rcdzc = [ "cadenza-ast" "cadenza-syntax" "cdz-num" "cdz-rt" "cdz-run" "rcdzc" ];
              cadenza-syntax = [ "cadenza-ast" "cadenza-syntax" ];
              cdz-num = [ "cdz-num" ];
              xtask = [ "cdz-rust-render" "xtask" ];
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

        # Operator seq-144 ("get ALL the reducer stuff set up on nix"): run the agent-harness bootstrap
        # reducers' behavioral @tests through nix, per-input-cached (v-harness-bootstrap owns the fixtures).
        # SEED-COMPILER path (`cdz test` on the project), INDEPENDENT of the kernel host adapter — the
        # heap-value @tests resolve the value-heap runtime by hash from my componentStore via CDZ_STORE
        # (set by testCadenzaProject). (The COMPONENT build + REDUCER_CADENZA_COMPONENT env — the
        # kernel-e2e half — is separate; the env wiring is adapter-gated on v-agent-harness.)
        #   🪤 ROOT AT THE PROJECT DIR: testCadenzaProject runs `cdz test .` at the unpacked-src cwd with
        #   the explicit "no upward manifest search" guard — so Project.cdz MUST be at the src root. A
        #   repo-root fileset.toSource would nest it 6 dirs down (Project.cdz not at `.` → the guarded
        #   search, github-liaison #2182). So root reducerCadenzaTestSrc AT the fixture dir (Project.cdz +
        #   the reducer .cdz land at top level) — same rooting as exampleProjectTests. `cdz test` needs no
        #   wit/ (the @tests are pure cdz-test, no component-target metadata — Project.cdz names no wit).
        reducerCadenzaTestSrc = pkgs.lib.fileset.toSource {
          root = ./implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-cadenza;
          fileset = ./implementation/seed/crates/cdz-kernel/tests/fixtures/reducer-cadenza;
        };
        reducerCadenzaTests = testCadenzaProject {
          pname = "cdz-reducer-cadenza-tests";
          src = reducerCadenzaTestSrc;
        };

        # seq-144 Part 2: compile a single Cadenza reducer .cdz to a wasm COMPONENT via the seed compiler.
        # `cdz compile --target wasm --component-name <name>` emits a component DIRECTLY (no cargo / no
        # vendor / no `wasm-tools component new` lift — unlike the Rust guests) — so this is seedCompiler-only
        # + fully hermetic (compile records the runtime import BY HASH; it does NOT need the store at build,
        # only at RUN — v-harness-bootstrap verified). The content hash falls out of the built bytes (same
        # shape as reducer-guest/cedar-guest: no committed pin). All B1-B4 reducers export
        # `cadenza:agent-kernel/fold` (genesis is an ordinary fold, not a separate world — v-hb confirmed).
        # b1/b2 import just the value-heap runtime by hash; b3/genesis also import cadenza:agent-kernel/kv
        # (host-served, unresolved at build — `wasm-tools validate` still passes, verified).
        # GUARD-BY-CONSTRUCTION (v-nix 2026-08-09): mkCadenzaComponent takes NO `outputHash` — a
        # reducer-cadenza component can NOT be made a FIXED-OUTPUT derivation, by design. WHY: every
        # component this maker compiles embeds its runtime DEPENDENCY pin (the compiler bakes the current
        # REQUIRED_RUNTIME_HASH into the emitted `cadenza:runtime/heap@…+<hash>` import). A FOD's output PATH
        # is keyed on the hand-pinned hash, NOT its inputs, so on a runtime-ABI bump nix serves the STALE
        # cached component (old runtime pin) while componentStore rebuilds to the new hash → the consumer's
        # get_by_hash(<old>) → BlobMissing (this exact bug reded cdz-agent-host-native's genesis E2E on
        # v-runtime's B0; the sibling cdzWasmPkg FOD reded guide-examples the same way — both reverted to
        # input-addressed, 6b894c84b + f2ab207de). The "FOD fails loud on a stale pin" guard only fires on a
        # COLD build; WARM it serves the stale path SILENTLY — the no-workaround-directive booby-trap. So
        # these stay input-addressed unconditionally: seedCompiler is in nativeBuildInputs → a hash bump
        # rotates seedCompiler → rotates the derivation → rebuilds → emits the CURRENT-runtime pin. The
        # per-MR-throughput early-cutoff a FOD gave (skip a consumer's 60s+ E2E on unrelated compiler edits)
        # must be recovered — if at all — via a content-addressed derivation (keys the path on the ACTUAL
        # built bytes → byte-identical early-cutoff AND correct rebuild-on-bump, which a fixed pin
        # structurally cannot), NOT a hand-pinned outputHash. Dropping the param makes reintroducing the bug
        # an eval error (unexpected argument), not a silent runtime BlobMissing.
        # `witWorld` (null | "pure" | "full"): when set, the reducer TARGETS a WIT world — materialize the
        # world artifact via the `emit-wit-world` bin and pass it as the `wit-world:reducer-world=<path>`
        # compile input (KIND_WIT_WORLD), so rcdzc's world-driven emit bytes-wraps the reducer to a
        # bytes-provider component (DESIGN-binary-ast-abi §3b). Absent (B1-B3/genesis) → the ordinary
        # handle-shaped fold emit, byte-identical to before.
        mkCadenzaComponent = { name, cdzFile, componentName ? "cadenza:agent-kernel/fold", contentAddressed ? false, witWorld ? null }:
          pkgs.stdenvNoCC.mkDerivation ({
            pname = name;
            version = "0.0.0";
            src = reducerCadenzaTestSrc; # fixture-dir-rooted → the reducer .cdz are at the src top level.
            nativeBuildInputs = [ seedCompiler ];
            buildPhase = ''
              runHook preBuild
              export HOME="$TMPDIR/home"; mkdir -p "$HOME"
              ${pkgs.lib.optionalString (witWorld != null) ''
              # Materialize the ${witWorld}-fold WIT world (KIND_WIT_WORLD binary-AST) via the prebuilt
              # emit-wit-world bin, then feed it as the wit-world:reducer-world input below.
              ${emitWitWorld} ${witWorld} world.bin
              ''}
              # compile the single reducer .cdz → a wasm component (emitted to component.wasm in the cwd).
              cdz compile ${cdzFile}${pkgs.lib.optionalString (witWorld != null) " wit-world:reducer-world=world.bin"} --target wasm --component-name ${componentName} -o component.wasm
              runHook postBuild
            '';
            # match the flake's other single-wasm derivations (reducer-guest/rcdzc-wasm): write $out in
            # installPhase, not the buildPhase (github-liaison #2182 consistency).
            installPhase = ''
              runHook preInstall
              cp component.wasm "$out"
              runHook postInstall
            '';
            # 🪤 dontFixup: $out is a single wasm FILE. stdenv's fixupPhase runs `strip` on file outputs,
            # which truncates a wasm to ~54 bytes → a corrupt component in the store (the SAME trap rcdzcWasm
            # guards; see its note "with fixup out=54B"). It's currently latent here — nativeBuildInputs is
            # seedCompiler-only, no binutils strip in PATH, so fixup's strip is a no-op (components build
            # intact: b1=497B). But that's INCIDENTAL: add a toolchain to nativeBuildInputs (or a stdenv
            # default shift) and strip silently corrupts. Make the guard explicit, same as rcdzcWasm
            # (github-liaison #2196 review).
            dontFixup = true;
          } // pkgs.lib.optionalAttrs contentAddressed {
            # CONTENT-ADDRESSED (v-nix 2026-08-09, CI-wall-time lever): key the output PATH on the actual
            # emitted bytes, not the input drv. WHY (vs input-addressed): this component is compiled by
            # seedCompiler, whose drv rotates on ANY compiler-CLOSURE edit (rcdzc changes ~every commit). An
            # INPUT-addressed output path therefore rotates on every such edit, so a heavy consumer (the
            # genesis E2E in cdz-agent-host-native, ~10-12m) cache-MISSES + rebuilds on a large fraction of
            # fleet MRs — the batch-gate long pole. But the emitted component is byte-IDENTICAL across a
            # compiler edit that doesn't change the emit (EMPIRICALLY VERIFIED 2026-08-09: an inert rcdzc
            # doc-comment edit rotated the drv 9lxb5…→8wyzg06… but the output stayed byte-identical, sha256
            # 6c2c096d…, 2285 B). CA keys the consumer on THOSE bytes → it cache-HITS whenever the emit is
            # unchanged, and correctly REBUILDS only when a compiler change actually moves the emit. This is
            # the safe form of the early-cutoff the old FIXED-OUTPUT pin gave (byte-identical hit) WITHOUT the
            # stale-serve hazard (a real emit change produces new bytes → new path → consumer rebuilds; a
            # fixed pin instead served stale). Needs experimental-features ca-derivations (enabled fleet-wide
            # in nix.custom.conf, v-ft 2026-08-09). $out is a single wasm FILE → flat mode.
            __contentAddressed = true;
            outputHashMode = "flat";
            outputHashAlgo = "sha256";
          });
        reducerCadenzaB1 = mkCadenzaComponent { name = "reducer-cadenza-b1"; cdzFile = "reducer_b1.cdz"; };
        reducerCadenzaB2 = mkCadenzaComponent { name = "reducer-cadenza-b2"; cdzFile = "reducer_b2.cdz"; };
        reducerCadenzaB3 = mkCadenzaComponent { name = "reducer-cadenza-b3"; cdzFile = "reducer_b3.cdz"; };
        # GENESIS is CONTENT-ADDRESSED (contentAddressed = true). It is the sole reducer-cadenza component
        # injected into a heavy native check (cdz-agent-host-native's 60s+ genesis E2E), which cache-MISSED on
        # every compiler-closure edit while genesis was input-addressed (the seedCompiler drv rotates ~every
        # commit) — the batch-gate long pole (operator CI-wall-time directive). CA recovers the early-cutoff
        # SAFELY: cache-hit when the emit is byte-identical, rebuild when it genuinely changes — unlike the
        # old FIXED-OUTPUT pin (f2ab207de) which served a STALE component on v-runtime's B0 hash bump →
        # BlobMissing. Empirically proven output-stable across an inert rcdzc edit (see the maker note).
        # A1 (2026-08-12): genesis rewritten to the bytes fold boundary — single-Event apply, kv.put host-routed
        # (escapes as the world's kv import, host-fused), inline-structural EffectRequest return. So it now
        # TARGETS the FULL reducer world (fold.apply + kv), materialized via emit-wit-world "full" — witWorld =
        # "full" drives rcdzc's world-driven bytes emit + fuses the kv.put escape against the world's kv import.
        # Built WITHOUT the world it would DECLINE (the host-fused escape needs the world to fuse against), so
        # this witWorld arg is co-landed same-batch with the fixture rewrite (v-agent-harness). PUT-ONLY (unit
        # result) → unaffected by the kv.get option-result path.
        reducerCadenzaGenesis = mkCadenzaComponent {
          name = "reducer-cadenza-genesis";
          cdzFile = "reducer_genesis.cdz";
          witWorld = "full";
          contentAddressed = true;
        };
        # PURE-GENESIS (A1 bytes fold boundary, DESIGN-binary-ast-abi §3b): the smallest REAL Cadenza reducer
        # that exercises the WHOLE bytes round-trip — `apply(list<u8>) -> list<u8>` decoding a value-form Event
        # doc and encoding a value-form effect-list doc — by TARGETING the pure-fold WIT world (fold.apply only,
        # NO kv import). `witWorld = "pure"` materializes that world (186 B) via emit-wit-world + drives rcdzc's
        # world-driven bytes emit, so the built component exports `cadenza:agent-kernel/fold`'s
        # apply(list<u8>)->list<u8> and imports ONLY cadenza:runtime/heap (verified: no kv). Consumed by the
        # cdz-agent-host pure-genesis E2E (host.rs real_pure_reducer_folds_an_event_through_the_a1_bytes_boundary)
        # via env PURE_GENESIS_REDUCER_COMPONENT, exported in agentHostEnvSetup below (the E2E resolves the heap
        # import from CDZ_STORE). CA for the same batch-gate early-cutoff reason as genesis (byte-identical hit
        # across inert compiler edits).
        reducerCadenzaPureGenesis = mkCadenzaComponent {
          name = "reducer-cadenza-pure-genesis";
          cdzFile = "reducer_pure.cdz";
          componentName = "cadenza:agent-kernel/fold";
          witWorld = "pure";
          contentAddressed = true;
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

        # Full-CI-in-nix increment 4: the GHA `cdz-kernel` job (cargo test + clippy + fmt + the
        # `--features live-exec` clippy/test) as a nix check. cdz-kernel is its OWN root-excluded
        # [workspace] (path-deps cadenza-ast), so it vendors from its OWN committed Cargo.lock (158 pkgs,
        # v-agent-harness `5a8bb10b0`). The CI job builds + validates the reducer-guest wasm then feeds it
        # via REDUCER_GUEST_COMPONENT — but my `reducerGuest` derivation ALREADY produces a validated
        # component, so the check just points the env at it (skips the redundant build+validate; the
        # component-model validity is separately gated by checks.reducer-guest-valid). The base `cargo
        # test` is hermetic + passes without the env (the component e2e is env-gated); passing it makes
        # that e2e actually RUN. Advisory-by-omission → unilateral cargo-twin retire once green.
        cdzKernelVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-kernel/Cargo.lock;
        };
        cdzKernelSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-kernel
            ./implementation/seed/crates/cadenza-ast
            ./rust-toolchain.toml
          ];
        };
        cdzKernelNativeCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-kernel-native";
          version = "0.0.0";
          src = cdzKernelSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = cdzKernelVendor; }}
            cd implementation/seed/crates/cdz-kernel
            # Feed the pre-built, pre-validated reducer-guest component (my derivation) so the env-gated
            # component-reducer e2e RUNS instead of skipping.
            export REDUCER_GUEST_COMPONENT="${reducerGuest}"
            # seq-144 last piece: feed the compiled B1 CADENZA reducer component + the value-heap store so
            # reducer_cadenza_b1_e2e.rs drives a REAL rcdzc-compiled reducer through apply_handle_lowered
            # (asserts vec-len==0, empty effects) instead of skipping. b1 imports the value-heap runtime, so
            # CDZ_STORE (my hash-keyed componentStore ROOT DIR: <sha256hex>.wasm blobs + runtime.toml) is
            # REQUIRED — the transitive nfc dep resolves by name via runtime.toml. The test FAILS LOUD if
            # the reducer needs its heap but no store is provided (a silent skip there would hide broken
            # wiring). Both come from my derivations (already built + validated). v-harness-bootstrap owns the
            # fixture; v-agent-harness owns the test (landed on trunk); the env export is mine.
            export REDUCER_CADENZA_COMPONENT="${reducerCadenzaB1}"
            # B2 climb: feed the compiled B2 reducer so reducer_cadenza_b2_e2e.rs RUNS (asserts ONE Http
            # effect to https://ok.host/x + correlation step-1) instead of skipping. Same sync
            # apply_handle_lowered path + CDZ_STORE transitive-nfc resolution as b1.
            export REDUCER_CADENZA_B2_COMPONENT="${reducerCadenzaB2}"
            # B3 climb (seq-144 reducer-e2e tail CLOSE): feed the compiled B3 reducer so
            # reducer_cadenza_b3_e2e.rs RUNS (drives a 'message' event through async apply_handle_lowered;
            # asserts kv['count']==[1] — b3's bound handle-ABI kv.get(None→0)+put through the marshalled
            # boundary, the first real-reducer KV-WRITE e2e — AND one Http effect to ok.host/x correlation
            # step-1) instead of skipping. Same apply_handle_lowered path + CDZ_STORE transitive-nfc
            # resolution as b1/b2. Closes the tail: b1+b2+b3+genesis all live in CI.
            export REDUCER_CADENZA_B3_COMPONENT="${reducerCadenzaB3}"
            export CDZ_STORE="${componentStore}"
            cargo test --locked
            cargo clippy --all-targets --locked -- -D warnings
            cargo fmt --check
            cargo clippy --all-targets --locked --features live-exec -- -D warnings
            cargo test --locked --features live-exec
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-kernel native (test + clippy + fmt + live-exec)" > "$out"
            runHook postInstall
          '';
        };

        # Full-CI-in-nix increment 5: the GHA `cdz-agent-host` job (cargo test + clippy + fmt, plus the
        # `--features admin` test + the default/admin/live-net/admin,live-net clippy matrix). cdz-agent-host
        # is its OWN root-excluded [workspace] (path-deps cdz-kernel → cadenza-ast), vendoring from its OWN
        # committed Cargo.lock (309 pkgs, v-agent-harness-host). Feeds BOTH guest components from my
        # derivations — CEDAR_POLICY_COMPONENT=${cedarGuest} (cedar authz e2e) + CDZ_REDUCER_COMPONENT=
        # ${reducerGuest} (ComponentSessionFactory e2e) — so the env-gated e2es RUN (unset → they skip).
        # Both are pre-validated by my derivations, so the check skips the CI job's build+validate-guest
        # steps. Advisory-by-omission → unilateral cargo-twin retire once green.
        cdzAgentHostVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-agent-host/Cargo.lock;
          # cdz-agent-host gained an ALWAYS-ON git dependency (v-agent-harness-host #2084, operator
          # seq-115/116: use s2n-quic's dc-metrics directly for histograms/reporting). importCargoLock
          # can't fetch a git source hermetically without its tree hash, so pin it here. The KEY is
          # "<name>-<version>" from the lock (NOT the git URL — see nixpkgs import-cargo-lock.nix); the
          # VALUE is the git-tree hash for the pinned rev (7ec9f027…, `nix flake prefetch git+…?rev=`).
          # A rev bump (the manifest is branch=main, though the lock pins the rev) → re-prefetch + re-pin.
          outputHashes = {
            "s2n-quic-dc-metrics-0.76.0" = "sha256-48vWZq7OSZcH1vf8qqH+DwnRyc+sH/BnJ+AhS6QrHBA=";
          };
        };
        cdzAgentHostSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-agent-host
            ./implementation/seed/crates/cdz-kernel
            ./implementation/seed/crates/cadenza-ast
            ./rust-toolchain.toml
          ];
        };
        # Uses the FULL stdenv (a C toolchain) + cmake + pkg-config, NOT stdenvNoCC: the `live-net`
        # feature closure pulls aws-lc-sys (the aws-sdk/reqwest rustls-tls default crypto provider), whose
        # build script drives a C/asm build via cmake. It happens to take a no-cmake path on this
        # version/target today (the check built green under stdenvNoCC), but that's fragile — a future
        # aws-lc-sys bump could require the C build, silently redding this check. Providing cmake +
        # pkg-config + a cc up front makes it robust regardless of aws-lc-sys's build path (github-liaison
        # #2018). Build tools only — this changes THIS derivation's store-path/hash (as any input change
        # does in Nix), but produces no DOWNSTREAM consumed artifact (it's a lint+test check).
        # 2-WAY PARALLEL SPLIT (v-nix, operator 12-MRs/hr throughput target 2026-08-09; concierge + v-ft
        # signed off). cdz-agent-host-native was the batch-gate LONG POLE (~10-12m): ONE derivation ran 8
        # sequential cargo passes (test/clippy/fmt × default/admin/live-net/admin,live-net feature combos).
        # Split into TWO derivations that nix builds CONCURRENTLY → wall ≈ max(half) not sum(8). Partitioned
        # along the DEPENDENCY-divergence boundary (the key design point): the `live-net` feature closure
        # pulls the heavy aws-sdk/aws-lc-sys(cmake) chain, default/admin do NOT (admin adds only tokio
        # subfeatures). So the CORE half (default + admin) never compiles the aws chain, and the LIVE-NET
        # half compiles it once — each half stays internally sequential (shares ONE target/ like today, so
        # no dep-recompile blowup), and only 2 run concurrently (well under v-ft's NIX_GATE_MAX_JOBS=4
        # box-saturation cap — a 4-way per-feature split would 4× the aws dep-compile + risk the cap). BOTH
        # halves fold into localGate (v-ft constraint 1: no split-out check left un-gated) + expose as
        # checks.<sys>; both are the AARCH64 variants (constraint 3). Coverage is IDENTICAL — the same 8
        # passes, repartitioned. The env-gated e2e wiring (guest components + CDZ_STORE) is shared via the
        # `agentHostEnvSetup` preamble so both halves run the same e2es they did before the split. Before/
        # after gate-wall-time read from pr-sync's --json blocks bracketing this land (measure-after, per
        # concierge's ruling — the split is coverage-identical so correctness is baseline-independent).
        agentHostEnvSetup = ''
          # cdz-agent-host has a GIT dependency (s2n-quic-dc-metrics) — mkCargoVendorEnv's default
          # (merged = false) sources the vendor's own config.toml, which carries the git source-
          # replacement stanza, so the offline build resolves the git crate from the vendor.
          ${mkCargoVendorEnv { vendor = cdzAgentHostVendor; }}
          cd implementation/seed/crates/cdz-agent-host
          # Feed the pre-built, pre-validated guest components (my derivations) so the env-gated cedar
          # authz + ComponentSessionFactory e2es RUN instead of skipping.
          export CEDAR_POLICY_COMPONENT="${cedarGuest}"
          export CDZ_REDUCER_COMPONENT="${reducerGuest}"
          # signature-query part-1 E2E (#2711): reflect the lifted cadenza:syntax component; feed syntaxGuest
          # so the CDZ_SYNTAX_COMPONENT-gated reflect E2E RUNS (unset → skips).
          export CDZ_SYNTAX_COMPONENT="${syntaxGuest}"
          # signature-query part-2 compose E2E (v-ah-host): feed the lifted consumer so the
          # CDZ_SYNTAX_CONSUMER_COMPONENT-gated compose E2E RUNS (skip-when-either-unset).
          export CDZ_SYNTAX_CONSUMER_COMPONENT="${consumerGuest}"
          # seq-144 genesis tail: feed the compiled GENESIS Cadenza reducer + the value-heap store so
          # v-ah-host's genesis round-trip E2E (host.rs real_genesis_reducer_folds_setup_events…) RUNS. The
          # genesis reducer imports the value-heap runtime + transitive nfc, resolved by hash/name from
          # CDZ_STORE (my hash-keyed componentStore). Distinct var from CDZ_REDUCER_COMPONENT.
          export GENESIS_REDUCER_COMPONENT="${reducerCadenzaGenesis}"
          # A1 pure-genesis (co-land step 2/3 completion): feed the precompiled PURE reducer bytes-provider
          # component so v-ah-host's real_pure_reducer_folds_an_event_through_the_a1_bytes_boundary E2E
          # (host.rs) RUNS instead of skipping — driving apply across the A1 bytes boundary
          # (build_event_document → guest → parse_effect_list) through the real component. The e2e gates on
          # BOTH this env AND CDZ_STORE (below): "pure" = no kv / no host caps, but the reducer still imports
          # cadenza:runtime/heap (it builds structural records + String.to-bytes), resolved from the store
          # like the genesis reducer's deps. Its List-rooted result value-encodes BARE (rcdzc result+param
          # value-form both bare after 50b82e141 + 01c29a0f5), which parse_effect_list accepts.
          export PURE_GENESIS_REDUCER_COMPONENT="${reducerCadenzaPureGenesis}"
          # CDZ_STORE resolves the genesis reducer's value-heap runtime + transitive nfc.
          export CDZ_STORE="${componentStore}"
        '';
        # The helper: one native-check half. `pname`/`passes` differ; everything else (src, the full stdenv
        # C toolchain + cmake for aws-lc-sys, the env preamble) is shared. See the split note above.
        mkAgentHostNative = { pname, passes }: pkgs.stdenv.mkDerivation {
          inherit pname;
          version = "0.0.0";
          src = cdzAgentHostSrc;
          nativeBuildInputs = [ rustToolchain pkgs.cmake pkgs.pkg-config ];
          # cmake is here for aws-lc-sys's build script to CALL, not to configure THIS derivation (no
          # CMakeLists.txt) — disable cmake's configure setup-hook so it doesn't hijack configurePhase.
          dontUseCmakeConfigure = true;
          buildPhase = ''
            runHook preBuild
            ${agentHostEnvSetup}
            ${passes}
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: ${pname}" > "$out"
            runHook postInstall
          '';
        };
        # CORE half: default + admin feature-sets (NO aws-sdk/live-net deps → no cmake-C aws-lc-sys build).
        cdzAgentHostNativeCore = mkAgentHostNative {
          pname = "cdz-agent-host-native-core";
          passes = ''
            cargo test --locked
            cargo clippy --all-targets --locked -- -D warnings
            cargo fmt --check
            cargo test --locked --features admin
            cargo clippy --all-targets --locked --features admin -- -D warnings
          '';
        };
        # LIVE-NET half: the live-net + admin,live-net feature-sets (pulls the aws-sdk/aws-lc-sys chain once).
        # Runs CONCURRENTLY with the core half. The live-net TEST (below) COMPILES the live-net test targets
        # (the real GAP-2 coverage — a live-net-only compile break like the CanonicalResolver E0433 reds
        # here); the network tests are ENV-GATED (CDZ_LIVE_HTTP_URL / AWS creds) so they SKIP without egress.
        cdzAgentHostNativeLiveNet = mkAgentHostNative {
          pname = "cdz-agent-host-native-live-net";
          passes = ''
            cargo clippy --all-targets --locked --features live-net -- -D warnings
            cargo clippy --all-targets --locked --features admin,live-net -- -D warnings
            cargo test --locked --features live-net
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
        mkStripComponent = { pname, crateDir, artifact, src, vendor, features ? [ ], emitRaw ? false }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";
            outputs = if emitRaw then [ "out" "raw" ] else [ "out" ];

            nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ];

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
              wasm-tools strip -a \
                target/wasm32-unknown-unknown/release/${artifact}.wasm \
                -o "$out"
              runHook postInstall
            '';
          };

        # The value-heap runtime derivations bind mkStripComponent to the cdz-runtime crate.
        mkRuntime = { pname, features, emitRaw ? false }:
          mkStripComponent {
            inherit pname features emitRaw;
            crateDir = "cdz-runtime";
            artifact = "cdz_runtime";
            src = runtimeSrc;
            vendor = runtimeVendor;
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

        # ── emit-wit-world: the pure/full WIT-world binary-AST materializer (cdz-kernel tool) ──────
        #
        # A small dedicated derivation building JUST the `emit-wit-world` bin (cdz-kernel/src/bin), which
        # writes a `KIND_WIT_WORLD` binary-AST world artifact (from the shared `cadenza-ast` builders) that
        # `rcdzc` ingests as the `wit-world:reducer-world=<path>` input when precompiling a reducer to a
        # bytes-provider component (DESIGN-binary-ast-abi §3b, pure-genesis co-land). Kept OUT of the
        # seedCompiler set (it is a kernel tool, not the compiler; v-agent-harness agreed 2026-08-12).
        #   - cdz-kernel is ROOT-EXCLUDED from the seed [workspace] with its OWN empty [workspace] + own
        #     Cargo.lock (like reducer-guest/cedar-guest), so this builds standalone from the crate dir,
        #     NOT `-p cdz-kernel` from the repo root (which fails — not a workspace member).
        #   - it path-deps ONLY `cadenza-ast` (a leaf, no further path deps), so the fileset is exactly
        #     the cdz-kernel dir + the cadenza-ast dir + rust-toolchain.toml; registry deps vendor from
        #     cdz-kernel's own Cargo.lock via importCargoLock.
        #   - 🪤 HEAVY BUILD: cdz-kernel UNCONDITIONALLY deps `wasmtime = "37"` + cranelift ("NOT optional
        #     — the kernel's engine"), so even this tiny bin compiles the whole wasmtime/cranelift tree.
        #     Pure-Rust (no aws-lc/ring/cmake — stdenvNoCC + rustToolchain suffices), but it rebuilds
        #     whenever cdz-kernel's closure rotates. That is why it feeds only the (rebuild-anyway) genesis
        #     component build, NOT the per-MR hot path directly.
        emitWitWorldVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-kernel/Cargo.lock;
        };
        emitWitWorldSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-kernel
            ./implementation/seed/crates/cadenza-ast
            ./rust-toolchain.toml
          ];
        };
        emitWitWorld = pkgs.stdenvNoCC.mkDerivation {
          pname = "emit-wit-world";
          version = "0.0.0";
          src = emitWitWorldSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = emitWitWorldVendor; }}
            cd implementation/seed/crates/cdz-kernel
            cargo build --release --locked --offline --bin emit-wit-world
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp target/release/emit-wit-world "$out"
            runHook postInstall
          '';
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
            # Under the BYTES fold boundary the guest speaks `cadenza-ast` DIRECTLY (decodes the event
            # doc + encodes the effect-list doc — DESIGN-binary-ast-abi §3d), so the guest crate carries a
            # `cadenza-ast` PATH dep; the offline --locked build needs its source in the fileset to resolve
            # the manifest (registry deps come from the guest's own Cargo.lock via importCargoLock).
            ./implementation/seed/crates/cadenza-ast
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
            ${mkCargoVendorEnv { vendor = reducerGuestVendor; }}
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
            ${mkCargoVendorEnv { vendor = cedarGuestVendor; }}
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

        # N2 (v-syntax P2, 2026-08-07): the cadenza:syntax REDUCER-FACING WIT component — a wit-bindgen
        # guest over cadenza-syntax exporting parse/query/doc, so a reducer can import it by content-hash
        # and compose_dep_into_linker links it leaves-first (v-agent-harness-host's compose-consumption
        # E2E consumes it via CDZ_SYNTAX_COMPONENT=${syntaxGuest}, mirroring reducer/cedar). Same shape as
        # reducer-guest/cedar-guest: own [workspace] + committed Cargo.lock → reproducible content-hash, no
        # committed .wasm pin. Build+lift+validate mirrors v-syntax's `syntax-guest` CI job (checks.yml):
        # `cargo build --locked --target wasm32-unknown-unknown --release` → `wasm-tools component new`.
        syntaxGuestVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-syntax-guest/Cargo.lock;
        };
        syntaxGuestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          # cdz-syntax-guest has PATH-deps on sibling crates (unlike cedar-guest which is self-contained):
          # cdz-syntax-guest → cadenza-syntax → cadenza-ast (both first-party path deps). The fileset MUST
          # include that closure or the offline build fails "failed to load manifest for cadenza-syntax"
          # (v-syntax's CI job works because a full-repo checkout has them; a scoped nix fileset must list
          # them). cadenza-ast is a leaf (no further path deps).
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-syntax-guest
            ./implementation/seed/crates/cadenza-syntax
            ./implementation/seed/crates/cadenza-ast
            ./rust-toolchain.toml
          ];
        };
        syntaxGuest = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-syntax-guest-component";
          version = "0.0.0";
          src = syntaxGuestSrc;
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = syntaxGuestVendor; }}
            cd implementation/seed/crates/cdz-syntax-guest
            cargo build --release --target wasm32-unknown-unknown --locked --offline
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            wasm-tools component new \
              target/wasm32-unknown-unknown/release/cdz_syntax_guest.wasm \
              -o "$out"
            runHook postInstall
          '';
        };

        # N2 part-2 (v-syntax #2673-consumer, v-ah compose E2E): the CONSUMER guest that IMPORTS
        # cadenza:syntax as a content-addressed +hash dep and calls parse.read-sexpr in its fold (the
        # flavor-2 direct-linked-cross-component demo). v-ah ruled option-i: THIS derivation templates
        # syntaxGuest's resolved content-hash into the consumer's WIT import name at BUILD time (the
        # committed source stays a `+SYNTAXGUESTHASH` placeholder). Mirrors rcdzc injecting the runtime
        # hash into an emitted program's import (mod.rs:7801) + our syntaxGuest content-addressing.
        # compose_dep_into_linker strips @ver+hash and matches only the bare `cadenza:syntax/parse`
        # (v-ah wasm_host.rs:366), so the +hash is provenance hygiene, NOT a compose-match requirement —
        # but templating the REAL hash is the right convention. No first-party path-deps (pure wit-bindgen
        # guest), so the fileset is just the crate dir (simpler than syntaxGuest's sibling closure).
        consumerGuestVendor = pkgs.rustPlatform.importCargoLock {
          lockFile = ./implementation/seed/crates/cdz-syntax-consumer-guest/Cargo.lock;
        };
        consumerGuestSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates/cdz-syntax-consumer-guest
            ./rust-toolchain.toml
          ];
        };
        consumerGuest = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-syntax-consumer-guest-component";
          version = "0.0.0";
          src = consumerGuestSrc;
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.coreutils pkgs.b3sum ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = consumerGuestVendor; }}
            cd implementation/seed/crates/cdz-syntax-consumer-guest
            # Template the RESOLVED syntaxGuest content-hash into the consumer's cadenza:syntax import
            # name (option-i). The placeholder `+SYNTAXGUESTHASH` appears in BOTH the import
            # (wit/consumer.wit) AND the vendored dep package decl (wit/deps/syntax/syntax.wit); both MUST
            # get the SAME hash or wit resolution rejects the import-vs-package mismatch. --replace-fail
            # aborts the build if the token is absent (never silently ship an un-templated placeholder).
            #
            # BLAKE3, NOT sha256 (v-ah ruling 2026-08-08): the composed WIT dep +hash is the kernel BLOB-STORE
            # KEY. resolve_deps (wasm_host.rs) reads this hex off the import name and does blobs.get(hash), and
            # the store keys by cdz_kernel::hash::Hash::of = blake3 (hash.rs:27). So the import MUST carry the
            # blake3 Hash::of of the dep bytes, or resolve_deps hits DepMissing (a sha256 hex can never match a
            # blake3-keyed store). This is a DIFFERENT address space from the on-disk sha256 CDZ_STORE
            # (REQUIRED_RUNTIME_HASH / packages.syntax-guest-hash) — the documented dual-hash boundary
            # (hash.rs:9-14): CDZ_STORE = sha256, kernel blob store (composed deps) = blake3. They never cross.
            # b3sum --no-names → the same lowercase 64-char hex Hash::to_hex produces.
            h=$(b3sum --no-names ${syntaxGuest})
            substituteInPlace wit/consumer.wit wit/deps/syntax/syntax.wit \
              --replace-fail "+SYNTAXGUESTHASH" "+$h"
            cargo build --release --target wasm32-unknown-unknown --locked --offline
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            wasm-tools component new \
              target/wasm32-unknown-unknown/release/cdz_syntax_consumer_guest.wasm \
              -o "$out"
            runHook postInstall
          '';
        };

        # The content address of a built component = blake3 of its (stripped) bytes. DERIVED from the
        # artifact nix built — this is the Cadenza content-address a program pins, falling out of the
        # build rather than being asserted. Exposed as a `packages.*-hash` (a plain-text store file).
        # BLAKE3 content-address (dual-hash COLLAPSE, operator ruling 2026-08-08: unify tree-wide on blake3 for
        # speed). Was sha256sum; the kernel's Hash::of is blake3, so every content-address — *-hash packages,
        # componentStore filenames, runtime.toml, the parity checks vs REQUIRED_RUNTIME_HASH, and the compose-dep
        # import (already blake3) — is now the same blake3 Hash::of. b3sum --no-names → the lowercase 64-char hex
        # Hash::to_hex produces. Co-lands with v-ah's component_store Hash::of flip + v-rust-backend's codegen
        # regen of REQUIRED_RUNTIME_HASH/DEBUG_RUNTIME_HASH (else the parity checks red on the stale sha256 constant).
        hashOf = drv: name:
          pkgs.runCommand name { } ''
            ${pkgs.b3sum}/bin/b3sum --no-names ${drv} | ${pkgs.coreutils}/bin/tr -d '\n' > $out
          '';

        # ── R2: the content-addressed component STORE ─────────────────────────────────────────────
        #
        # Assemble every nix-built component into ONE store directory, each file named by its DERIVED
        # content hash: `<blake3>.wasm`. This mirrors `target/cadenza-store` (what `xtask build`
        # produces) but built + addressed BY NIX — the store the operator's north star describes, from
        # which a cadenza runtime / the harness loads a component by hash. Purely a function of the
        # component derivations, so it's cache-shareable + rebuilt only when a component changes.
        # (A later increment has the runtime/harness RESOLVE from this store; that's a cross-territory
        # change coordinated with v-runtime + the harness — this increment only PRODUCES the store.)
        # BLAKE3 (dual-hash collapse) — filenames + runtime.toml use b3sum, matching the kernel Hash::of.
        componentStore = pkgs.runCommand "cdz-component-store" { } ''
          set -euo pipefail
          mkdir -p "$out"
          for c in ${runtime} ${runtimeDebug} ${nfc} ${reducerGuest} ${cedarGuest} ${syntaxGuest}; do
            h=$(${pkgs.b3sum}/bin/b3sum --no-names "$c")
            ${pkgs.coreutils}/bin/cp "$c" "$out/$h.wasm"
          done
          # `cdz-run` resolves the runtime's NFC dependency (FINDING#23) by reading `runtime.toml` from the
          # store (the `nfc = "<hash>"` line → `<store>/<hash>.wasm`), and the runtime/debug hashes from
          # it too — WITHOUT this manifest every heap case that composes the runtime fails to resolve NFC.
          # `xtask build` writes exactly this file (main.rs:466); mirror its format so a program run against
          # THIS nix store composes identically to one run against target/cadenza-store.
          rt=$(${pkgs.b3sum}/bin/b3sum --no-names ${runtime})
          dbg=$(${pkgs.b3sum}/bin/b3sum --no-names ${runtimeDebug})
          nfc=$(${pkgs.b3sum}/bin/b3sum --no-names ${nfc})
          cat > "$out/runtime.toml" <<EOF
          # Cadenza content-addressed store — the value-heap runtime + its NFC dependency.
          runtime = "$rt"
          debug_runtime = "$dbg"
          nfc = "$nfc"
          EOF
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
        # `.#syntax-guest` = the lifted cadenza:syntax component; `.#syntax-guest-hash` its derived content
        # address (v-agent-harness-host's compose-test imports it by this hash). N2, v-syntax P2.
        packages.syntax-guest = syntaxGuest;
        packages.syntax-guest-hash = hashOf syntaxGuest "syntax-guest-hash";
        # `.#syntax-consumer-guest` = the lifted part-2 consumer component (imports cadenza:syntax by
        # +hash, calls parse.read-sexpr); `.#syntax-consumer-guest-hash` its content address. N2 part-2.
        packages.syntax-consumer-guest = consumerGuest;
        packages.syntax-consumer-guest-hash = hashOf consumerGuest "syntax-consumer-guest-hash";

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

        # seq-144: the agent-harness bootstrap reducers' @tests through nix (`.#reducer-cadenza-tests`).
        packages.reducer-cadenza-tests = reducerCadenzaTests;

        # seq-144 Part 2: the B1-B4 reducer wasm COMPONENTS (cdz-compiled) + their derived content hashes.
        # `.#reducer-cadenza-b1` … `-genesis`; `-hash` each (the address the kernel store/e2e loads by).
        packages.reducer-cadenza-b1 = reducerCadenzaB1;
        packages.reducer-cadenza-b2 = reducerCadenzaB2;
        packages.reducer-cadenza-b3 = reducerCadenzaB3;
        packages.reducer-cadenza-genesis = reducerCadenzaGenesis;
        packages.reducer-cadenza-pure-genesis = reducerCadenzaPureGenesis;
        packages.emit-wit-world = emitWitWorld;
        packages.reducer-cadenza-b1-hash = hashOf reducerCadenzaB1 "reducer-cadenza-b1-hash";
        packages.reducer-cadenza-b2-hash = hashOf reducerCadenzaB2 "reducer-cadenza-b2-hash";
        packages.reducer-cadenza-b3-hash = hashOf reducerCadenzaB3 "reducer-cadenza-b3-hash";
        packages.reducer-cadenza-genesis-hash = hashOf reducerCadenzaGenesis "reducer-cadenza-genesis-hash";
        packages.reducer-cadenza-pure-genesis-hash = hashOf reducerCadenzaPureGenesis "reducer-cadenza-pure-genesis-hash";

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
                got=$(${pkgs.b3sum}/bin/b3sum --no-names ${drv})
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
              clippy-cdz-corpus = mkCrateClippyCrane { crate = "cdz-corpus"; extraSrc = [ ./spec/semantics ]; };
              clippy-cdz-num = mkCrateClippyCrane { crate = "cdz-num"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              clippy-cdz-rt = mkCrateClippyCrane { crate = "cdz-rt"; };
              clippy-cdz-run = mkCrateClippyCrane { crate = "cdz-run"; extraSrc = [ ./implementation/compiler-ml ]; };
              clippy-cdz-rust-render = mkCrateClippyCrane { crate = "cdz-rust-render"; };
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
              test-cdz-corpus = mkCrateTestCrane { crate = "cdz-corpus"; extraSrc = [ ./spec/semantics ]; };
              test-cdz-num = mkCrateTestCrane { crate = "cdz-num"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              test-cdz-rt = mkCrateTestCrane { crate = "cdz-rt"; };
              test-cdz-run = mkCrateTestCrane { crate = "cdz-run"; extraSrc = [ ./implementation/compiler-ml ]; };
              test-cdz-rust-render = mkCrateTestCrane { crate = "cdz-rust-render"; };
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
                inherit (perCrateClippyCrane) clippy-rcdzc clippy-cdz-num clippy-cdz-calc clippy-cadenza-syntax;
              } ''
              echo "ok: clippy shard A — rcdzc + cdz-num + cdz-calc + cadenza-syntax" > $out
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
            #   · component VALIDITY (guest + reducer-cadenza components are well-formed wasm components).
            #   · the project @test suites run through nix (example-project + reducer-cadenza b1-b4/genesis).
            #   · the pure-eval closure-assert guard.
            # The advisory job runs `nix build .#checks.<sys>.flake-repro-backstop` (minutes, cache-warm) instead
            # of `nix flake check` (48m). Coverage of the required set is unchanged (those jobs still run); only
            # the redundant re-run is dropped. `nix flake check` locally/in devShell still builds everything.
            flakeReproBackstop = pkgs.runCommand "flake-repro-backstop"
              {
                inherit runtimeHashParity runtimeDebugHashParity nfcHashParity
                  reducerGuestValid cedarGuestValid
                  reducerCadenzaB1Valid reducerCadenzaB2Valid reducerCadenzaB3Valid reducerCadenzaGenesisValid
                  reducerCadenzaPureGenesisValid
                  exampleProjectTests reducerCadenzaTests crateClosureAssert;
              } ''
              echo "ok: flake reproducibility-backstop — hash-parity + component-validity + project-@tests + closure-assert" > $out
            '';
            # bindings the backstop aggregate references (kept as `let` so both the aggregate + the individual
            # `checks.*` attrs below share ONE derivation each — no rebuild).
            runtimeHashParity = parity { name = "runtime"; drv = runtime; constName = "REQUIRED_RUNTIME_HASH"; };
            runtimeDebugHashParity = parity { name = "runtime-debug"; drv = runtimeDebug; constName = "DEBUG_RUNTIME_HASH"; };
            nfcHashParity = parity { name = "nfc"; drv = nfc; constName = "REQUIRED_NFC_HASH"; };
            reducerGuestValid = validComponent { name = "reducer-guest"; drv = reducerGuest; };
            cedarGuestValid = validComponent { name = "cedar-guest"; drv = cedarGuest; };
            syntaxGuestValid = validComponent { name = "syntax-guest"; drv = syntaxGuest; };
            syntaxConsumerGuestValid = validComponent { name = "syntax-consumer-guest"; drv = consumerGuest; };
            reducerCadenzaB1Valid = validComponent { name = "reducer-cadenza-b1"; drv = reducerCadenzaB1; };
            reducerCadenzaB2Valid = validComponent { name = "reducer-cadenza-b2"; drv = reducerCadenzaB2; };
            reducerCadenzaB3Valid = validComponent { name = "reducer-cadenza-b3"; drv = reducerCadenzaB3; };
            reducerCadenzaGenesisValid = validComponent { name = "reducer-cadenza-genesis"; drv = reducerCadenzaGenesis; };
            reducerCadenzaPureGenesisValid = validComponent { name = "reducer-cadenza-pure-genesis"; drv = reducerCadenzaPureGenesis; };

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
            # The three ADVISORY natives (cdz-kernel/cdz-agent-host/cad-tests) are NOT in ruleset-10, so they
            # are deliberately EXCLUDED from the aggregate's fail-set (a red on them must not block merge,
            # matching prod). They stay independently buildable + warm via their own `checks.*` attrs; pr-sync
            # can build them separately for extra signal without gating. FAIL-CLOSED: the aggregate depends on
            # all 9 required, so `nix build` of it is red if ANY required check fails — no silent gap. aarch64.
            localGate = pkgs.runCommand "local-gate"
              {
                # The 9 merge-required-minus-macos contexts PLUS the two workspace-ISOLATED native checks
                # (cdz-agent-host-native, cdz-kernel-native). Those crates are excluded from the cargo
                # workspace (so the macOS workspace test never reds on them) and were therefore NOT in this
                # aggregate — gate-local skipped them, forcing pr-sync's --local-gate recipe to bolt on a
                # manual `cargo test --manifest-path …` per crate. Folding them in makes the aggregate cover
                # them (one green/red, hermetic + cached) and drops that manual step (v-nix+v-ft 2026-08-08).
                # Both resolve the per-arch value-heap runtime hash from CDZ_STORE, so they MUST be the
                # aarch64 derivations (#2348) — localGate IS the aarch64 aggregate, so that holds by construction.
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
                  cdzAgentHostNativeCore cdzAgentHostNativeLiveNet cdzKernelNativeCheck mandateLintCheck;
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
              echo "ok: local-gate — 9 merge-required contexts (ruleset-10 minus test-macos) + cdz-agent-host/kernel-native + mandate-lint + cad-test-compiler-ml (Core-shape spine guard), green on aarch64-nix" > $out
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
            reducer-guest-valid = reducerGuestValid;
            cedar-guest-valid = cedarGuestValid;
            syntax-guest-valid = syntaxGuestValid;
            syntax-consumer-guest-valid = syntaxConsumerGuestValid;
            # S3: the example project's @tests run through nix — a cache HIT when its sources are
            # unchanged (the "skip tests that haven't changed" win), a re-run + fail on a red test.
            example-project-tests = exampleProjectTests;
            # seq-144: agent-harness bootstrap reducer @tests through nix (b1/b2/b3/genesis — 14 @tests).
            reducer-cadenza-tests = reducerCadenzaTests;
            # seq-144 Part 2: each B1-B4 reducer component is a valid wasm component (b3/genesis import kv
            # host-served/unresolved — validate checks STRUCTURE not import-satisfaction, so still green).
            reducer-cadenza-b1-valid = reducerCadenzaB1Valid;
            reducer-cadenza-b2-valid = reducerCadenzaB2Valid;
            reducer-cadenza-b3-valid = reducerCadenzaB3Valid;
            reducer-cadenza-genesis-valid = reducerCadenzaGenesisValid;
            reducer-cadenza-pure-genesis-valid = reducerCadenzaPureGenesisValid;

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
            # Full-CI-in-nix increment 4: the GHA cdz-kernel job (test + clippy + fmt + live-exec).
            cdz-kernel-native = cdzKernelNativeCheck;
            # Full-CI-in-nix increment 5: the GHA cdz-agent-host job (test + clippy + fmt + feature matrix).
            cdz-agent-host-native-core = cdzAgentHostNativeCore;
            cdz-agent-host-native-live-net = cdzAgentHostNativeLiveNet;
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
          // cdzCadProjectTests;

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
                #  - the guest wasm COMPONENTS (cedar/reducer/syntax) — cargo-wasm build-inputs of
                #    cdz-agent-host-native / cdz-kernel-native, NOT in those checks' runtime closure, so
                #    without their own root the GC drops them and a candidate rebuilds the heavy cedar-policy
                #    dep chain (~60s) cold (v-agent-harness-host ask 2026-08-09).
                # --out-link registers each as an indirect GC-root so the store stays hot.
                nix build \
                  ".#packages.${system}.cargo-artifacts" \
                  ".#packages.${system}.cargo-artifacts-release" \
                  ".#packages.${system}.cargo-artifacts-release-codegen" \
                  ".#packages.${system}.store" \
                  ".#packages.${system}.cedar-guest" \
                  ".#packages.${system}.reducer-guest" \
                  ".#packages.${system}.syntax-guest" \
                  ".#checks.${system}.local-gate" \
                  --out-link "$root_dir/warm" --print-build-logs
                echo "cdz warm-keep: done — local /nix/store warm layer pinned (gate-local stays fast)."
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
      });
}
