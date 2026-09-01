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

  # SHARED binary cache (operator-greenlit 2026-08-30, "the cachix token is already in the CI, give it a shot").
  # The `camshaft` cachix cache is the durable root fix for the "rebuilding the whole world" cold-corpus / gate-
  # local starvation: CI PUSHES the first-party build outputs (compiler + corpus + cdz-wasm closures) here via
  # cachix-action (CACHIX_AUTH_TOKEN, already in CI — v-gha-green wires the CI push side), and every agent + CI
  # job PULLS them as substitutes instead of cold-rebuilding from source. PULL needs only the public key below
  # (no token — read-only); only PUSH needs the token. This SIDESTEPS the disabled cache-nix-action daemon-DB-
  # swap regression entirely (a substituter, not a store-DB merge → no daemon race). Untrusted-user builds
  # IGNORE this config with a warning (no hard fail); trusted users + `accept-flake-config` honor it. Restores
  # the prior-art wiring from #144 / the removed reference/ subsystem (commit 0d625573aa).
  nixConfig = {
    extra-substituters = [ "https://camshaft.cachix.org" ];
    extra-trusted-public-keys = [ "camshaft.cachix.org-1:vJQM+N6ilGdDPFSUiH0tL5pBLZ/cD4acir7t4I2zGSc=" ];
  };

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
        # The TEST-RUNNER variant (seedCompilerTestRunner, --features cdz/standalone) links rcdzc IN-PROCESS
        # for `cdz test`/discovery/emit-shred — so its SRC fileset MUST carry rcdzc's src. seedCompilerClosure
        # is computed STATICALLY (independent of --features), and once v-cdz-crate-split flips rcdzc to
        # `optional=true` (standalone=["dep:rcdzc"]) its includeOptional=false will DROP rcdzc-src → the
        # test-runner build would fail "no rcdzc src" despite --features standalone. So explicitly UNION rcdzc's
        # own closure in. TODAY (rcdzc still a non-optional cdz dep) this is a NO-OP — rcdzc is already in
        # seedCompilerClosure — so it's safe to pre-land; it just SURVIVES the optional flip. corpus/other
        # optionals stay OUT (includeOptional=false on both), preserving the test-runner's leanness. The
        # compile/delegate seedCompiler keeps the plain seedCompilerClosure and CORRECTLY sheds rcdzc post-flip.
        seedTestRunnerClosure = pkgs.lib.unique
          (seedCompilerClosure ++ crateClosure' { includeOptional = false; } "rcdzc");
        seedTestRunnerSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (pkgs.lib.concatMap crateCompileSrc seedTestRunnerClosure)
            ++ nonClosureManifests seedTestRunnerClosure
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
        # The seed cdz/cdz-run build, PARAMETERIZED by feature set. Two variants share ALL machinery (src, deps
        # layer, hash-injection preBuild, non-closure stubs) and differ ONLY in cargoExtraArgs:
        #  - seedCompiler (COMPILE/DELEGATE) — `--no-default-features` → `standalone` OFF, so `cdz compile`/`cdz
        #    build` DELEGATE to the external `cdz-compile` (v-cdz-delegate's caching win: a compiler change need
        #    not rebuild `cdz`). Also sheds corpus/lsp/watch/completions from the closure.
        #  - seedCompilerTestRunner (TEST/DISCOVERY) — `--features cdz/standalone` → `standalone` ON, so `cdz
        #    test` / `cdz test --list` / `cdz test --emit-shred` run the compiler + property-gen IN-PROCESS via
        #    rcdzc (`run_test` is `#[cfg(feature="standalone")]`-only, main.rs:5426). This is the cdz the cad-test
        #    + test-shred-discovery derivations MUST use — the `!standalone` seedCompiler's `cdz test` REFUSES
        #    ("no in-process test runner"). corpus/lsp/watch/completions stay OFF (cdz test is corpus-independent).
        #    rcdzc is already in seedCompilerClosure (non-optional today); WHEN v-cdz-crate-split flips
        #    `standalone=["dep:rcdzc"]` + `rcdzc={optional=true}`, THIS variant's closure must gain
        #    includeOptional for rcdzc (seam coordinated with v-cdz-crate-split — the compile variant stays as-is).
        mkSeedCompiler = { pname, cargoExtraArgs, src ? seedCompilerSrc, closure ? seedCompilerClosure }: craneLib.buildPackage {
          inherit pname cargoExtraArgs src;
          version = "0.0.0";
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
            ${stubNonClosure closure}
            [ -f xtask/src/main.rs ] || { mkdir -p xtask/src; echo "fn main(){}" > xtask/src/main.rs; }
            [ -f xtask/src/lib.rs ] || echo "" > xtask/src/lib.rs
          '';
          # Build only the seed-compiler binaries, not the whole workspace (xtask etc.). crane injects --locked
          # + --release; the per-variant cargoExtraArgs adds the -p scoping + feature set (see mkSeedCompiler note).
          # Build only — tests run in the existing gate/CI (S1: reproducible toolchain build). Do NOT re-export
          # the deps layer (we consume the shared cargoArtifacts, not produce a new one).
          doCheck = false;
          doInstallCargoArtifacts = false;
        };
        # COMPILE/DELEGATE variant (standalone OFF). `--no-default-features` drops cdz's default-on `corpus`
        # (v-nix+v-cml 2026-08-10) — paired with seedCompilerClosure's includeOptional=false (drops cdz-corpus
        # SRC), a corpus-only MR no longer rotates seedCompiler; and it sheds lsp/watch/completions. cdz-run has
        # no default features, so --no-default-features is a no-op for it. This cdz DELEGATES `cdz compile`/`build`
        # to cdz-compile (v-cdz-delegate). NOTE: it CANNOT run `cdz test` (standalone-gated → the honest refusal
        # stub) — the cad-test / test-shred-discovery derivations use seedCompilerTestRunner instead.
        seedCompiler = mkSeedCompiler {
          pname = "cdz-seed-compiler";
          cargoExtraArgs = "-p cdz -p cdz-run --no-default-features";
        };
        # TEST/DISCOVERY variant (standalone ON) — the in-process rcdzc test runner for `cdz test` / `cdz test
        # --list` / `cdz test --emit-shred`. Kept SEPARATE from seedCompiler so the compile-delegation caching win
        # is preserved. `--features cdz/standalone` flips ON the standalone cfg-gates while keeping
        # corpus/lsp/watch/completions OFF (standalone=[] today, so it pulls no extra deps beyond the already-in-
        # closure rcdzc). Consumed by mkCadProjectTest, testCadenzaProject, testDiscovery, mkTestShred.
        seedCompilerTestRunner = mkSeedCompiler {
          pname = "cdz-seed-compiler-testrunner";
          cargoExtraArgs = "-p cdz -p cdz-run --no-default-features --features cdz/standalone";
          # Dedicated src/closure that explicitly carries rcdzc (see seedTestRunnerClosure) — survives the
          # rcdzc-optional flip; no-op today. The compile seedCompiler above keeps the default closure.
          src = seedTestRunnerSrc;
          closure = seedTestRunnerClosure;
        };

        # xtaskBin — the `xtask` dev-tool binary AS a relocatable nix package (v-xtask-decompose, operator
        # all-nix mandate 2026-08-28: decompose the xtask monolith into per-subcommand nix apps agents run
        # via `nix run .#<cmd>` — no bare `cargo`). Mirrors `seedCompiler`: warm cargoArtifacts + pinned
        # seedCargoVendor + the non-closure stub preBuild, scoped to `-p xtask` so it reuses the same
        # ~383MB dep-closure layer instead of a cold per-worktree rebuild. Output: `$out/bin/xtask`. The
        # per-subcommand apps (`apps.roundtrip`, &c.) wrap this + set `CDZ_REPO_ROOT` so the relocated
        # binary self-locates the invoking worktree (xtask's `Paths::resolve` bakes CARGO_MANIFEST_DIR,
        # which a nix build points at the sandbox — the env override is the relocatability seam). Src via
        # `craneCrateCommon { crate = "xtask"; }` — xtask's dep-CLOSURE fileset with xtask's real src
        # present + the other members stubbed (seedCompilerSrc would instead STUB xtask itself, building an
        # empty bin). `doCheck = false` = build the binary only (tests run in the per-crate test-xtask check).
        # NON-BREAKING: the existing `cargo xtask …` path (env unset) is unchanged; this builds the SAME
        # binary alongside it.
        xtaskBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask"; }) // {
          pname = "cdz-xtask";
          cargoExtraArgs = "-p xtask";
          doCheck = false;
        });

        # The standalone corpus round-trip command as its own relocatable crane bin (v-xtask-decompose):
        # only xtask-roundtrip + xtask-support compile (no xtask), so it caches independently. `apps.roundtrip`
        # runs it with CDZ_SEED_BIN_DIR (nix-built cdz/cdz-corpus). Output: $out/bin/xtask-roundtrip.
        xtaskRoundtripBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-roundtrip"; }) // {
          pname = "cdz-xtask-roundtrip";
          cargoExtraArgs = "-p xtask-roundtrip";
          doCheck = false;
        });

        # xtaskMandatesBin — the STANDALONE mandate-lint binary (v-xtask-decompose). Built from ONLY the
        # xtask-mandates crate's closure (`craneCrateCommon { crate = "xtask-mandates"; }` → src is just
        # that crate + its sole dep syn), so it caches INDEPENDENTLY of xtask (operator 2026-08-28: "we
        # only get the wins if xtask doesn't have a direct dependency on these subcrates — cache each
        # subcrate independently"). `apps.lint-mandates` wraps it; the mandate GATE runs it too. `$out/bin/
        # xtask-mandates`.
        xtaskMandatesBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-mandates"; }) // {
          pname = "cdz-xtask-mandates";
          cargoExtraArgs = "-p xtask-mandates";
          doCheck = false;
        });

        # xtaskLintEmojiBin — the STANDALONE emoji-ban source lint (v-xtask-decompose). Built from ONLY the
        # xtask-lint-emoji crate's closure (deps just xtask-support → cdz-contract → cadenza-ast), so it caches
        # INDEPENDENTLY of xtask (operator 2026-08-28: "cache each subcrate independently"). `apps.lint-emoji`
        # wraps it with CDZ_REPO_ROOT. Output: $out/bin/xtask-lint-emoji.
        xtaskLintEmojiBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-lint-emoji"; }) // {
          pname = "cdz-xtask-lint-emoji";
          cargoExtraArgs = "-p xtask-lint-emoji";
          doCheck = false;
        });

        # xtaskCanonicalizeBaselinesBin — the STANDALONE `.gate-baseline*` canonicalizer (v-xtask-decompose).
        # Built from ONLY the xtask-canonicalize-baselines crate's closure (deps just xtask-support → the
        # std-only baseline-text algebra → cdz-contract → cadenza-ast), so it caches INDEPENDENTLY of xtask
        # (operator 2026-08-28: "cache each subcrate independently"). `apps.canonicalize-baselines` wraps it
        # with CDZ_REPO_ROOT. Output: $out/bin/xtask-canonicalize-baselines.
        xtaskCanonicalizeBaselinesBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-canonicalize-baselines"; }) // {
          pname = "cdz-xtask-canonicalize-baselines";
          cargoExtraArgs = "-p xtask-canonicalize-baselines";
          doCheck = false;
        });

        # xtaskFmtBin — the STANDALONE Cadenza formatter (v-xtask-decompose). Built from ONLY the xtask-fmt
        # crate's closure (deps just xtask-support → cdz-contract → cadenza-ast), so it caches INDEPENDENTLY
        # of xtask. `apps.fmt` wraps it with CDZ_SEED_BIN_DIR (the nix-built cdz) so it runs cargo-free.
        # Output: $out/bin/xtask-fmt.
        xtaskFmtBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-fmt"; }) // {
          pname = "cdz-xtask-fmt";
          cargoExtraArgs = "-p xtask-fmt";
          doCheck = false;
        });

        # cdzWorldArtifactBin — the WIT-world → KIND_WIT_WORLD binary-AST utility as a relocatable crane bin
        # (v-xtask-decompose). The cdz-world-artifact crate ALREADY holds the logic (deps just cadenza-ast);
        # this replaces `cargo xtask world-artifact`'s `cargo build -p cdz-world-artifact` shell-out (a bare
        # cargo call the operator all-nix mandate forbids) with a warm crane bin `apps.world-artifact` runs.
        # Output: $out/bin/cdz-world-artifact. (The `worldArtifacts` build derivation is unchanged — it builds
        # the crate in its own scoped derivation; this bin is just the local-dev convenience entry point.)
        cdzWorldArtifactBin = craneLib.buildPackage ((craneCrateCommon { crate = "cdz-world-artifact"; }) // {
          pname = "cdz-world-artifact";
          cargoExtraArgs = "-p cdz-world-artifact";
          doCheck = false;
        });

        # xtaskCodegenContractsBin — the contract-schema projector (v-xtask-decompose, codegen→build-time-nix).
        # Carved from codegen.rs's generate_contracts (RENDER path only). Built from its own closure (deps
        # cadenza-ast + cdz-contract + the syn/quote/prettyplease render stack), so it caches independently of
        # xtask. A `cdzPlatformContracts` derivation (v-nix, mirroring contractHashes) runs this over the
        # contract sources with the seed cdz + component store to EMIT cdz-platform/src/contracts/*.rs at build
        # time — a build-phase overlay then copies them in, so nothing generated is committed. Output:
        # $out/bin/xtask-codegen-contracts. Verified: emits BYTE-IDENTICAL output to the committed contracts.
        xtaskCodegenContractsBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-codegen-contracts"; }) // {
          pname = "cdz-xtask-codegen-contracts";
          cargoExtraArgs = "-p xtask-codegen-contracts";
          doCheck = false;
        });

        # xtaskCodegenWasmAbiBin — the backend wasm/component-model byte-table extractor (v-xtask-decompose,
        # codegen→build-time-nix). Carved from codegen.rs's generate_wasm_abi + its wasm_abi module. PURE
        # data extraction from wasm-encoder (no compiler dep, no cdz, no store, no runtime hash), so its
        # closure is just itself + the wasm-encoder/syn/quote/prettyplease deps — caches independently. A
        # `cdzWasmAbi` derivation runs it to EMIT rcdzc/src/backend/wasm/wasm_abi.rs at build time (overlay
        # copies it in). Output: $out/bin/xtask-codegen-wasm-abi. Verified: BYTE-IDENTICAL to the committed file.
        xtaskCodegenWasmAbiBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-codegen-wasm-abi"; }) // {
          pname = "cdz-xtask-codegen-wasm-abi";
          cargoExtraArgs = "-p xtask-codegen-wasm-abi";
          doCheck = false;
        });
        # xtaskCodegenDeclinesBin (v-deferral-declines seq-106; v-nix flake reg) — generates
        # rcdzc/src/diag/declines_generated.rs (the DeclineId catalog) FROM data/unsupported.sexp. Mirrors
        # xtaskCodegenWasmAbiBin: crane-built from its own closure (cadenza-ast + external syn/quote/prettyplease).
        xtaskCodegenDeclinesBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-codegen-declines"; }) // {
          pname = "cdz-xtask-codegen-declines";
          cargoExtraArgs = "-p xtask-codegen-declines";
          doCheck = false;
        });

        # xtaskCodegenGuideBin — the guide sexp→TSX codegen (v-guide-infra I5, the whole-guide→sexpr flip;
        # v-nix owns the nix wiring). Mirrors xtaskCodegenWasmAbiBin: crane-built from its OWN closure
        # (cadenza-syntax-sexpr + cadenza-syntax-core + cadenza-ast), so it caches independently of xtask.
        # `guideExamplesCheck` sets CDZ_XTASK_CODEGEN_GUIDE=${this}/bin/xtask-codegen-guide so the guide's
        # `npm run codegen` uses the PREBUILT bin (guideExamplesCheck has rustToolchain but NO cargo vendor, so
        # an in-gate `cargo build -p` can't run offline — v-guide-infra's scripts resolve the env-bin when set,
        # else fall back to `cargo build -p` for local dev). Output: $out/bin/xtask-codegen-guide.
        xtaskCodegenGuideBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-codegen-guide"; }) // {
          pname = "cdz-xtask-codegen-guide";
          cargoExtraArgs = "-p xtask-codegen-guide";
          doCheck = false;
        });

        # xtaskPruneBaselinesBin — the `.gate-baseline*` unreferenced-entry pruner (v-xtask-decompose). Built
        # from ONLY the xtask-prune-baselines crate's closure (deps xtask-support → cdz-contract → cadenza-ast),
        # so it caches INDEPENDENTLY of xtask. `apps.prune-baselines` runs it with CDZ_SEED_BIN_DIR (the
        # nix-built cdz-corpus for the corpus title set). Output: $out/bin/xtask-prune-baselines.
        xtaskPruneBaselinesBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-prune-baselines"; }) // {
          pname = "cdz-xtask-prune-baselines";
          cargoExtraArgs = "-p xtask-prune-baselines";
          doCheck = false;
        });

        # xtaskSaveBaselineBin — the thin `cargo xtask gate --save` replacement (v-xtask-decompose seq-202
        # gate-delete). Reads the `.#corpus-verdicts` harvest (<tag>\t<description> lines) into a
        # description→verdict map + writes .gate-baseline via `xtask_support::serialize_baseline`. Built from
        # ONLY its crate closure (deps xtask-support), so it caches independently of xtask. `apps.save-baseline`
        # builds the harvest + runs this bin `VERDICTS-FILE BASELINE-OUT`. Output: $out/bin/xtask-save-baseline.
        xtaskSaveBaselineBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-save-baseline"; }) // {
          pname = "cdz-xtask-save-baseline";
          cargoExtraArgs = "-p xtask-save-baseline";
          doCheck = false;
        });

        # xtaskBenchBin — the STANDALONE runtime allocation benchmark (v-xtask-decompose). Built from ONLY the
        # xtask-bench crate's closure (a std-only LEAF — no deps at all), so it caches INDEPENDENTLY of xtask.
        # `apps.bench` wraps it with CDZ_REPO_ROOT; the bin itself shells `cargo test` in cdz-runtime for the
        # measurement (so cargo must be on PATH at run time — same as the old `cargo xtask bench`). Output:
        # $out/bin/xtask-bench.
        xtaskBenchBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-bench"; }) // {
          pname = "cdz-xtask-bench";
          cargoExtraArgs = "-p xtask-bench";
          doCheck = false;
        });

        # xtaskInstallLspBin — the STANDALONE `install-lsp` command (v-xtask-decompose). Built from ONLY the
        # xtask-install-lsp crate's closure (a std+xshell LEAF — no workspace deps), so it caches independently
        # of xtask. `apps.install-lsp` wraps it with CDZ_REPO_ROOT (+ args passthrough for --uninstall). Output:
        # $out/bin/xtask-install-lsp.
        xtaskInstallLspBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-install-lsp"; }) // {
          pname = "cdz-xtask-install-lsp";
          cargoExtraArgs = "-p xtask-install-lsp";
          doCheck = false;
        });

        # xtaskDuvetCheckBin — the STANDALONE duvet citation-floor check (v-xtask-decompose). Built from ONLY
        # the xtask-duvet-check crate's closure (a serde_json+std LEAF). `apps.duvet-check` wraps it. Output:
        # $out/bin/xtask-duvet-check.
        xtaskDuvetCheckBin = craneLib.buildPackage ((craneCrateCommon { crate = "xtask-duvet-check"; }) // {
          pname = "cdz-xtask-duvet-check";
          cargoExtraArgs = "-p xtask-duvet-check";
          doCheck = false;
        });

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
        # roundtripCheck builds -p cdz -p cdz-corpus + runs xtask-roundtrip, so its true closure is those three
        # crates' dep-closures — NOT the whole workspace. seedRoundtripSrc (all crates) rotated the check on ANY
        # crate edit (MEASURED 2026-08-29: a cdz-platform edit — a HOT platform-push crate roundtrip does not
        # build — rotated it 3ws3jj19->rc2z93m7, forcing a from-scratch cdz-compiler rebuild + corpus round-trip
        # = pure waste). SCOPE it to the closure (full src) + Cargo.toml-only for the rest + spec/semantics (the
        # corpus xtask-roundtrip reads at runtime); the check stubs the non-closure members (stubClosure below)
        # so cargo still loads the workspace. Mirrors seedCompilerSrc/scopedToolSrc. Now a non-closure-crate
        # edit (cdz-platform, cdz-rust-*, cdz-cad, cdz-smith, …) no longer rotates roundtrip.
        roundtripClosure = pkgs.lib.unique (
          crateClosure "cdz" ++ crateClosure "cdz-corpus" ++ crateClosure "xtask-roundtrip");
        seedRoundtripSrcScoped = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (pkgs.lib.concatMap crateCompileSrc roundtripClosure)
            ++ nonClosureManifests roundtripClosure
            ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ./spec/semantics ]);
        };
        # crateCdzCheckSrc = seedTestSrc MINUS spec/semantics — the caching fast-path narrowing for the ONE
        # whole-workspace localGate constituent (crateCdzCheck). MEASURED (v-nix caching push 2026-08-29): a
        # 1-line corpus edit (spec/semantics/*.sexp — the HIGHEST-frequency fleet change) was rotating
        # crate-cdz (gby9sksi->48j2v1a8) and rebuilding the whole first-party workspace + rerunning cdz tests,
        # pure waste. crateCdzCheck runs `cargo build --workspace` (libs/bins only, NOT --all-targets) +
        # `cargo clippy -p cdz --all-targets` + `cargo test -p cdz` — NONE of which read spec/semantics: there
        # is NO build.rs and NO include_str!/include_bytes! of spec/semantics in any crate (verified), and cdz's
        # own inline tests are corpus-independent (v-cml). The only spec/semantics readers are cadenza-syntax's
        # corpus_roundtrip / markdown TESTS (via CARGO_MANIFEST_DIR at test-RUN time), which run under
        # test-cadenza-syntax / the workspace testCheck — NOT here. So dropping spec/semantics from THIS check's
        # src is coverage-neutral and stops corpus edits from rotating it. compiler-ml STAYS (cdz's run_ml_cli
        # tests read implementation/compiler-ml/src at test-run time — a legit rotation). The `cargo build
        # --workspace` STAYS too: cdz's run_rust_cli tests rustc-link the sibling cdz-num/cdz-rt rlibs the
        # full-workspace build lays out (a bare `-p cdz` -> E0433) — do NOT split to per-crate crane (concierge
        # ack notwithstanding: the split is unsafe for those tests; this surgical src-narrowing is the safe win).
        crateCdzCheckSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/seed/crates
            ./implementation/compiler-ml
            ./xtask
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo
            ./rust-toolchain.toml
          ];
        };
        cargoWorkspaceCheck = { name, cargoCmd, src ? seedSrc, extraInputs ? [ ], stubClosure ? null }:
          pkgs.stdenvNoCC.mkDerivation {
            pname = name;
            version = "0.0.0";
            inherit src;
            nativeBuildInputs = [ rustToolchain ] ++ extraInputs;
            buildPhase = ''
              runHook preBuild
              # stubClosure (v-nix caching 2026-08-29): when set, this check runs on a SCOPED src (full src for
              # the given dep-closure + Cargo.toml-only for the rest) — so an edit to a NON-closure crate does
              # NOT rotate it. cargo still LOADS the whole workspace (needs every member's src/lib.rs present),
              # so synthesize empty stubs for the non-closure members (the seedCompilerSrc/crane pattern). Only
              # opt-in checks pass stubClosure; the default (null) keeps the whole-tree behavior (fmt/crate-cdz).
              ${pkgs.lib.optionalString (stubClosure != null) ''
                chmod -R u+w .
                ${stubNonClosure stubClosure}
              ''}
              # #5250 flip: cdz-platform/src/contracts is build-time-generated (dropped from the committed
              # tree). ANY workspace-src check that COMPILES or FORMATS cdz-platform must see it or `mod
              # contracts;` (cdz-platform/src/lib.rs) fails to resolve — `cargo fmt --all` (fmtCheck) and
              # `cargo build --workspace` (crateCdzCheck) / `cargo test --workspace` (testCheck) all hit it.
              # The per-crate crane checks + platformItest already stage this overlay in their own preBuild;
              # the workspace-src checks were the missed sites (breaker gate-blocker: local-gate red at
              # cargo-fmt, masked earlier by an aborted-sibling clippy red). Stage the generated files here
              # too — they are cargo-fmt-clean (rendered via `rustfmt --edition 2024`, cdz-platform is
              # edition 2024, no rustfmt.toml), so `fmt --all --check` sees no diff. Guarded so a tree
              # without cdz-platform is a no-op.
              if [ -d implementation/seed/crates/cdz-platform/src ]; then
                chmod -R u+w implementation/seed/crates/cdz-platform/src
                mkdir -p implementation/seed/crates/cdz-platform/src/contracts
                cp ${cdzPlatformContracts}/contracts/*.rs implementation/seed/crates/cdz-platform/src/contracts/
              fi
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
          # cadenza-compile-abi (v-cdz-crate-split, approach-B extract): the dep-light compile-boundary types
          # (Target + OptLevel now; a later slice takes a single cadenza-ast dep for the Request/Query codec).
          # A ROOT workspace member (crates/* glob) → MUST be registered here or the crane deps-layer src omits
          # its Cargo.toml and the workspace fails to load. slice-1 is a pure-std LEAF (zero deps); rcdzc/cdz dep
          # it in v-cdz's slice-1b (lands AFTER this registration). Standalone/inert until then.
          cadenza-compile-abi = "implementation/seed/crates/cadenza-compile-abi";
          cadenza-syntax = "implementation/seed/crates/cadenza-syntax";
          # cadenza-syntax-* (v-syntax #5076/#5082): cadenza-syntax was split into a dependency-light bottom
          # (cadenza-syntax-core: spans + arena read-helpers + shared literal lexing) plus one crate per data
          # surface — json/sexpr/toml (always-on, re-exported) and cedar (feature-gated, isolates the heavy
          # cedar-policy dep off the front-end hot path). All are ROOT workspace members (crates/*, no own
          # [workspace]), so — like cdz-contract / cdz-world-artifact — each MUST be registered here or the
          # crane deps-layer src omits its Cargo.toml and the whole workspace fails to load (`cargo check`
          # can't read a member's manifest → cadenza-seed-deps-deps.drv fails → cascades fleet-wide).
          cadenza-syntax-cedar = "implementation/seed/crates/cadenza-syntax-cedar";
          cadenza-syntax-core = "implementation/seed/crates/cadenza-syntax-core";
          cadenza-syntax-json = "implementation/seed/crates/cadenza-syntax-json";
          cadenza-syntax-sexpr = "implementation/seed/crates/cadenza-syntax-sexpr";
          cadenza-syntax-toml = "implementation/seed/crates/cadenza-syntax-toml";
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
          # wasm-abi-table (v-xtask-decompose C2 option-B): the derived wasm/component byte-table as a
          # standalone LEAF crate (lib rlib; NO normal deps; dev-dep wasm-encoder ONLY). rcdzc will DEP it for
          # the constants + stay wasm-encoder-free. A ROOT workspace member (no own [workspace]), so — like
          # cdz-contract / cdz-world-artifact — it MUST be registered here or the crane deps-layer src omits
          # its Cargo.toml and the whole workspace fails to load. Its src/lib.rs is generated from
          # data/wasm-abi.sexp by `xtask-codegen-wasm-abi --crate-lib` (committed byte-identical for now).
          wasm-abi-table = "implementation/seed/crates/wasm-abi-table";
          rcdzc = "implementation/seed/crates/rcdzc";
          # rcdzc-cli (v-cdz-crate-split 2026-08-30): the clap arg-parsing layer + the `cdz-compile` bin,
          # extracted so the `rcdzc` compiler is a PURE LIBRARY with no clap (operator directive). A ROOT
          # workspace member (implementation/seed/crates/* glob, no own [workspace]), so — like the others —
          # it MUST be registered here or the crane deps-layer src omits its Cargo.toml and the workspace
          # fails to load. `cdzCompile` builds `-p rcdzc-cli --bin cdz-compile`.
          rcdzc-cli = "implementation/seed/crates/rcdzc-cli";
          xtask = "xtask";
          # xtask-mandates (v-xtask-decompose): the mandate-lint carved out of the xtask monolith into its
          # own minimal-dep crate (syn only). A ROOT workspace member (under the new `xtask/crates/*` glob,
          # no own [workspace]), so — like cdz-contract / cdz-world-artifact — it MUST be registered here or
          # the crane deps-layer src omits its Cargo.toml and the whole workspace fails to load.
          xtask-mandates = "xtask/crates/xtask-mandates";
          # xtask-support (v-xtask-decompose): the shared foundation lib for the decomposed xtask commands
          # (content_address/hash_tree now; corpus/Tools machinery to follow). A ROOT workspace member under
          # xtask/crates/*, so — like the others — it MUST be registered here or the crane deps-src omits its
          # Cargo.toml and the workspace fails to load.
          xtask-support = "xtask/crates/xtask-support";
          # xtask-roundtrip (v-xtask-decompose): the corpus round-trip check as its own bin crate, deps only
          # xtask-support. Registered here so the crane deps-src includes its Cargo.toml.
          xtask-roundtrip = "xtask/crates/xtask-roundtrip";
          # xtask-lint-emoji (v-xtask-decompose): the emoji-ban source lint as its own bin crate, deps only
          # xtask-support. Registered here so the crane deps-src includes its Cargo.toml.
          xtask-lint-emoji = "xtask/crates/xtask-lint-emoji";
          # xtask-canonicalize-baselines (v-xtask-decompose): the .gate-baseline* canonicalizer as its own
          # bin crate, deps only xtask-support. Registered here so the crane deps-src includes its Cargo.toml.
          xtask-canonicalize-baselines = "xtask/crates/xtask-canonicalize-baselines";
          # xtask-fmt (v-xtask-decompose): the Cadenza formatter as its own bin crate, deps only xtask-support.
          # Registered here so the crane deps-src includes its Cargo.toml.
          xtask-fmt = "xtask/crates/xtask-fmt";
          # xtask-prune-baselines (v-xtask-decompose): the .gate-baseline* pruner as its own bin crate, deps
          # only xtask-support. Registered here so the crane deps-src includes its Cargo.toml.
          xtask-prune-baselines = "xtask/crates/xtask-prune-baselines";
          # xtask-save-baseline (v-xtask-decompose seq-202 --save gate-delete): the thin `cargo xtask gate
          # --save` replacement — reads the .#corpus-verdicts harvest (<tag>\t<description> lines) + writes
          # .gate-baseline via serialize_baseline. Deps xtask-support only. Registered same-window with the
          # cherry-picked crate (a crate can't land without its flake reg — Cargo.lock vs rootWorkspaceCrates).
          xtask-save-baseline = "xtask/crates/xtask-save-baseline";
          # xtask-merge-baseline (v-xtask-decompose seq-202): the .gate-baseline* git MERGE DRIVER carved into
          # its own bin (v-ft repoints register_merge_drivers to it after this lands). Deps xtask-support only.
          xtask-merge-baseline = "xtask/crates/xtask-merge-baseline";
          # xtask-codegen-contracts (v-xtask-decompose): the contract-schema projector (codegen→build-time-nix),
          # deps cadenza-ast + cdz-contract. Registered here so the crane deps-src includes its Cargo.toml.
          xtask-codegen-contracts = "xtask/crates/xtask-codegen-contracts";
          # xtask-codegen-wasm-abi (v-xtask-decompose): the wasm/component byte-table extractor
          # (codegen→build-time-nix), deps only wasm-encoder (external). Registered so crane sees its Cargo.toml.
          xtask-codegen-wasm-abi = "xtask/crates/xtask-codegen-wasm-abi";
          xtask-codegen-declines = "xtask/crates/xtask-codegen-declines";
          # xtask-codegen-guide (v-guide-infra I5, whole-guide→sexpr flip; v-nix owns the nix wiring): the guide
          # sexp→TSX codegen (deps cadenza-syntax-sexpr + cadenza-ast). Registered so crane sees its Cargo.toml +
          # `xtaskCodegenGuideBin` builds it; guideExamplesCheck sets CDZ_XTASK_CODEGEN_GUIDE to the prebuilt bin.
          xtask-codegen-guide = "xtask/crates/xtask-codegen-guide";
          # xtask-bench (v-xtask-decompose): the runtime allocation benchmark carved out of xtask/src/bench.rs
          # into its own STD-ONLY leaf bin crate (NO deps — not even xtask-support). Registered here so the crane
          # deps-src includes its Cargo.toml (else the workspace fails to load). `benchCheck` runs it; the
          # xtask `Cmd::Bench` arm is removed so `cargo xtask bench` forwards to `apps.bench` (nix run .#bench).
          xtask-bench = "xtask/crates/xtask-bench";
          # xtask-install-lsp (v-xtask-decompose): the `install-lsp` command carved into its own std+xshell
          # LEAF crate. A ROOT workspace member (xtask/crates/* glob), so it MUST be registered here or the
          # crane deps-src omits its Cargo.toml → `cargo build --workspace` (crateCdzCheck) fails to load.
          xtask-install-lsp = "xtask/crates/xtask-install-lsp";
          # xtask-duvet-check (v-xtask-decompose): the duvet citation-floor check carved into its own
          # serde_json+std LEAF crate. A ROOT workspace member (xtask/crates/* glob) → MUST be registered.
          xtask-duvet-check = "xtask/crates/xtask-duvet-check";
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
        # scopedToolSrc — a SCOPED src for a plain-cargo (stdenvNoCC) TOOL build (`cargo build -p CRATE`):
        # FULL src/ for CRATE's dep-closure + Cargo.toml-only for every non-closure member (+ the buildPhase
        # writes synthetic stubs via `stubNonClosure (crateClosure crate)` so cargo's `members` glob parses)
        # + the pins + any `extra` runtime files the tool READS (e.g. a WIT dir). Replaces the broad
        # `platformItestSrc` (which unioned ./xtask + ALL seed crates + compiler-ml + spec/semantics) for the
        # pure-build tools — that breadth made ANY edit under those paths (xtask, a corpus .sexp, an unrelated
        # crate) spuriously rotate the tool, and worldArtifacts/cdzComponentRewrite feed the runtime component
        # → its hash → seedCompiler, so a single xtask edit rebuilt the whole compiler world + could flip
        # guide-examples (v-xtask-decompose 87ba0546, v-nix 2026-08-28). Mirrors seedCompilerSrc's isolation.
        scopedToolSrc = { crate, extra ? [ ] }: pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (
            (pkgs.lib.concatMap crateCompileSrc (crateClosure crate))
            ++ nonClosureManifests (crateClosure crate)
            ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]
            ++ extra);
        };
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
              ${pkgs.lib.optionalString (crate == "cdz-platform") ''
                # OVERLAY (v-nix, codegen→build-time-nix): stage the BUILD-TIME-generated contract schemas
                # over cdz-platform/src/contracts, so the generated files (not committed source) drive the
                # compile. cdz-platform's real src compiles ONLY here (clippy/test-cdz-platform — guarded to
                # this crate; other crates STUB cdz-platform) + platformItest. LOAD-BEARING since #5250 dropped
                # the committed src/contracts + gitignored them — this overlay is now the SOLE source of the
                # contract schemas for the compile (was a byte-identical no-op in the #5244 additive phase).
                mkdir -p implementation/seed/crates/cdz-platform/src/contracts
                cp ${cdzPlatformContracts}/contracts/*.rs implementation/seed/crates/cdz-platform/src/contracts/
              ''}
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
              # rcdzc's ONLY path-deps are its dev-deps cadenza-syntax + cdz-rt + cdz-num (#5000 DROPPED the
              # cdz-run dev-dep — wasmtime AND cdz-run are now fully out of the compiler crate; that also
              # removed cdz-contract + cdz-corpus-grade, which had entered only via cdz-run). The cadenza-syntax
              # split (#5076/#5082) then pulled the extracted surface crates into cadenza-syntax's closure —
              # cadenza-syntax-core (bottom) + json/sexpr/toml (always-on) + cadenza-syntax-cedar (optional, but
              # the default-true closure walk counts it) — each deps only cadenza-ast + cadenza-syntax-core, so
              # rcdzc reaches all five transitively through cadenza-syntax.
              # rcdzc gained a `cadenza-compile-abi` dep (v-cdz-crate-split slice-1b #5638 — the compile-boundary
              # Target/OptLevel types; slice-1b re-exports them from rcdzc; E2b adds the Request/Query codec).
              # cadenza-compile-abi's own closure is [cadenza-ast cadenza-compile-abi], but rcdzc already reaches
              # cadenza-ast (via cadenza-syntax), so it adds only cadenza-compile-abi itself to rcdzc's closure.
              rcdzc = [ "cadenza-ast" "cadenza-compile-abi" "cadenza-syntax" "cadenza-syntax-cedar" "cadenza-syntax-core" "cadenza-syntax-json" "cadenza-syntax-sexpr" "cadenza-syntax-toml" "cdz-num" "cdz-rt" "rcdzc" ];
              # rcdzc-cli (v-cdz-crate-split 2026-08-30): the clap CLI layer + `cdz-compile` bin. Its ONLY
              # first-party path-dep is `rcdzc` (clap + tracing-subscriber are external), so its closure is
              # rcdzc's closure ∪ {rcdzc-cli}. rcdzc's OWN closure is unchanged (rcdzc-cli deps rcdzc, not
              # the reverse; rcdzc dropped only the external clap/tracing-subscriber).
              rcdzc-cli = [ "cadenza-ast" "cadenza-compile-abi" "cadenza-syntax" "cadenza-syntax-cedar" "cadenza-syntax-core" "cadenza-syntax-json" "cadenza-syntax-sexpr" "cadenza-syntax-toml" "cdz-num" "cdz-rt" "rcdzc" "rcdzc-cli" ];
              cadenza-syntax = [ "cadenza-ast" "cadenza-syntax" "cadenza-syntax-cedar" "cadenza-syntax-core" "cadenza-syntax-json" "cadenza-syntax-sexpr" "cadenza-syntax-toml" ];
              cadenza-syntax-core = [ "cadenza-ast" "cadenza-syntax-core" ];
              cadenza-syntax-cedar = [ "cadenza-ast" "cadenza-syntax-cedar" "cadenza-syntax-core" ];
              cadenza-syntax-json = [ "cadenza-ast" "cadenza-syntax-core" "cadenza-syntax-json" ];
              cadenza-syntax-sexpr = [ "cadenza-ast" "cadenza-syntax-core" "cadenza-syntax-sexpr" ];
              cadenza-syntax-toml = [ "cadenza-ast" "cadenza-syntax-core" "cadenza-syntax-toml" ];
              cdz-num = [ "cdz-num" ];
              # cadenza-compile-abi deps cadenza-ast (default-features=false = the no_std core: the sidecar
              # Request/Query encode/decode codec builds on cadenza_ast::Builder + cadenza_ast::codec).
              # cadenza-ast is a foundational leaf (no workspace path-deps), so the closure is the two.
              cadenza-compile-abi = [ "cadenza-ast" "cadenza-compile-abi" ];
              # cdz-world-artifact deps only cadenza-ast (the language's binary-AST builders/codec) + the
              # external wit-parser; xtask still deps cadenza-ast via codegen.rs, so its closure is unchanged.
              cdz-world-artifact = [ "cadenza-ast" "cdz-world-artifact" ];
              # xtask does NOT depend on xtask-mandates — the dep was SEVERED (v-xtask-decompose 2026-08-28,
              # operator: cache each subcrate independently). The mandate lint is now the standalone
              # xtask-mandates crate + `apps.lint-mandates` + the rewired `mandateLintCheck`; nothing links
              # it into xtask, so editing one never rebuilds the other.
              # xtask-mandates now deps xtask-support (v-fleet-tooling 2026-08-30, v-xtask-decompose OPTION A):
              # the mandate binary runs `xtask_support::file_size_lint` (single source of truth with the
              # monolith), so the file-size mandate has teeth in localGate. That adds xtask-support's closure
              # (cadenza-ast + cdz-contract + xtask-support; sha2 is external) to xtask-mandates.
              # xtask now deps xtask-support (the shared foundation lib — content_address/hash_tree moved
              # there). xtask-support's own closure is cadenza-ast + cdz-contract + itself (cdz-contract deps
              # cadenza-ast; sha2 is external).
              # TRANSIENT closure growth (v-xtask-decompose): the cdz-corpus-grade dep (→ cadenza-syntax
              # family + cadenza-compile-abi) is a remove-at-gate-delete transient — the in-process
              # gate --check grader single-sources cdz_corpus_grade::canonical_output_value (SLICE 1, #7273
              # fix). When the gate-machinery delete lands, xtask stops grading in-process and this shrinks back.
              xtask = [ "cadenza-ast" "cadenza-compile-abi" "cadenza-syntax" "cadenza-syntax-cedar" "cadenza-syntax-core" "cadenza-syntax-json" "cadenza-syntax-sexpr" "cadenza-syntax-toml" "cdz-contract" "cdz-corpus-grade" "cdz-rust-render" "xtask" "xtask-support" ];
              xtask-support = [ "cadenza-ast" "cdz-contract" "xtask-support" ];
              # xtask-roundtrip deps xtask-support (which deps cdz-contract→cadenza-ast).
              xtask-roundtrip = [ "cadenza-ast" "cdz-contract" "xtask-roundtrip" "xtask-support" ];
              # xtask-lint-emoji deps xtask-support (which deps cdz-contract→cadenza-ast).
              xtask-lint-emoji = [ "cadenza-ast" "cdz-contract" "xtask-lint-emoji" "xtask-support" ];
              # xtask-canonicalize-baselines deps xtask-support (which deps cdz-contract→cadenza-ast).
              xtask-canonicalize-baselines = [ "cadenza-ast" "cdz-contract" "xtask-canonicalize-baselines" "xtask-support" ];
              # xtask-fmt deps xtask-support (which deps cdz-contract→cadenza-ast).
              xtask-fmt = [ "cadenza-ast" "cdz-contract" "xtask-fmt" "xtask-support" ];
              # xtask-prune-baselines deps xtask-support (which deps cdz-contract→cadenza-ast).
              xtask-prune-baselines = [ "cadenza-ast" "cdz-contract" "xtask-prune-baselines" "xtask-support" ];
              # xtask-save-baseline + xtask-merge-baseline dep xtask-support only (→ cdz-contract → cadenza-ast),
              # mirroring xtask-roundtrip.
              xtask-save-baseline = [ "cadenza-ast" "cdz-contract" "xtask-save-baseline" "xtask-support" ];
              xtask-merge-baseline = [ "cadenza-ast" "cdz-contract" "xtask-merge-baseline" "xtask-support" ];
              # xtask-codegen-contracts deps cadenza-ast + cdz-contract (cdz-contract deps cadenza-ast).
              xtask-codegen-contracts = [ "cadenza-ast" "cdz-contract" "xtask-codegen-contracts" ];
              # xtask-codegen-wasm-abi deps only external crates (wasm-encoder/syn/quote/prettyplease) — its
              # workspace closure is just itself.
              # now deps cadenza-ast (reads wasm-abi.sexp's cadenza-ast binary in the --from-sexpr producer).
              xtask-codegen-wasm-abi = [ "cadenza-ast" "xtask-codegen-wasm-abi" ];
              xtask-codegen-declines = [ "cadenza-ast" "xtask-codegen-declines" ];
              # xtask-codegen-guide (v-guide-infra I5): reads a chapter .sexp via the MAIN reader
              # (cadenza-syntax-sexpr → cadenza-syntax-core + cadenza-ast) into a cadenza-ast Arenas and emits
              # the @generated .tsx. Closure = the sexpr-reader stack + itself.
              xtask-codegen-guide = [ "cadenza-ast" "cadenza-syntax-core" "cadenza-syntax-sexpr" "xtask-codegen-guide" ];
              xtask-mandates = [ "cadenza-ast" "cdz-contract" "xtask-mandates" "xtask-support" ];
              # xtask-bench is a std-only leaf: NO workspace deps (not even xtask-support) — closure is just itself.
              xtask-bench = [ "xtask-bench" ];
              # xtask-install-lsp is a std+xshell LEAF: no workspace deps → closure is just itself.
              xtask-install-lsp = [ "xtask-install-lsp" ];
              # xtask-duvet-check is a serde_json+std LEAF: no workspace deps → closure is just itself.
              xtask-duvet-check = [ "xtask-duvet-check" ];
            };
            mismatches = builtins.filter (n: (crateClosure n) != expected.${n})
              (builtins.attrNames expected);
          in
          if mismatches != [ ] then
            throw ("flake.nix Part-B closure-assert: fromTOML closure walk disagrees with expected for "
              + builtins.toString mismatches
              + " — the crate dep graph changed; re-verify vs `cargo metadata` and update `expected`. ACTUAL: "
              + builtins.concatStringsSep " | " (map (n: n + "=[" + builtins.concatStringsSep " " (crateClosure n) + "]") mismatches))
          else
            pkgs.runCommand "crate-closure-assert" { } ''
              echo "ok: per-crate closures match expected (${builtins.toString (builtins.attrNames expected)})" > $out
            '';

        # cdz-run PATH-ONLY assert (v-cdz-crate-split, operator 2026-08-28): cdz-run holds wasmtime; NO
        # crate may link it as a LIBRARY (reached only via PATH — the cdz plugin dispatcher forwards). A lib
        # dependent rebuilds wasmtime constantly (seedCompiler does today via cdz). Pure-eval RATCHET: assert
        # the workspace-member crates depending on cdz-run are a SUBSET of `allowed`, shrunk to [] as the
        # crate-split severs each. A NEW dependent (or failure to shrink after severing) fails LOUD at eval.
        # (cdz-smith deps cdz-run too but is in a SEPARATE excluded workspace → not a rootCrateName → out of scope.)
        cdzRunDependentsAssert =
          let
            allowed = [ "cdz" "cdz-calc" ];  # DRIVE DOWN to [] as v-cdz-crate-split severs run/test/run-rust
            dependents = builtins.filter
              (c: c != "cdz-run" && builtins.elem "cdz-run" (crateDirectDeps { } c))
              rootCrateNames;
            illegal = builtins.filter (c: !(builtins.elem c allowed)) dependents;
          in
          if illegal != [ ] then
            throw ("flake.nix cdz-run-dependents-assert: " + builtins.toString illegal
              + " link `cdz-run` as a library, but cdz-run holds wasmtime and must be PATH-only. Forward to "
              + "the cdz-run binary instead, or (if intentional) add to `allowed` — goal is []. dependents: "
              + builtins.toString dependents + ".")
          else
            pkgs.runCommand "cdz-run-dependents-assert" { } ''
              echo "ok: cdz-run lib dependents subset of allowed (goal: none - PATH-only)" > $out
            '';

        # EXACTLY-ONE-WASMTIME-HOLDER assert (v-cdz-crate-split; operator: "i dont want two things to link
        # to wasmtime"). Complements cdzRunDependentsAssert (which guards LINKING cdz-run) — this guards a
        # NEW DIRECT wasmtime dep anywhere. Pure-eval: the workspace members with a NON-OPTIONAL `wasmtime`
        # in [dependencies] must be exactly [ cdz-run ]. cdz-platform's wasmtime is optional=true (host
        # feature, off in routine builds) -> excluded + a sanctioned 2nd integration. A new non-optional
        # wasmtime dep elsewhere fails LOUD at eval.
        wasmtimeSingleHolderAssert =
          let
            holdsWasmtime = name:
              let
                manifest = builtins.fromTOML
                  (builtins.readFile (./. + "/${rootWorkspaceCrates.${name}}/Cargo.toml"));
                w = (manifest.dependencies or { }).wasmtime or null;
              in
              w != null && !(builtins.isAttrs w && (w.optional or false));
            holders = builtins.filter holdsWasmtime rootCrateNames;
          in
          if holders != [ "cdz-run" ] then
            throw ("flake.nix wasmtime-single-holder-assert: non-optional `wasmtime` [dependencies] holders = "
              + builtins.toString holders + ", expected [ cdz-run ] — wasmtime must stay confined to cdz-run "
              + "(operator: one crate links wasmtime). cdz-platform's host-feature wasmtime is OPTIONAL "
              + "(excluded); a NEW non-optional wasmtime dep is a regression — drop it, reach the runner via "
              + "the cdz-run binary.")
          else
            pkgs.runCommand "wasmtime-single-holder-assert" { } ''
              echo "ok: non-optional wasmtime confined to cdz-run" > $out
            '';

        # COMPILER-IS-A-PURE-LIBRARY assert (v-cdz-crate-split, operator 2026-08-30: "the compiler should
        # not be pulling clap in as a dependency … the compiler should be a pure library"). The clap CLI
        # surface + trace SINK were extracted to `rcdzc-cli` (PR #6305); `rcdzc` itself must stay free of
        # BOTH `clap` (arg parsing) and `tracing-subscriber` (the sink — the lib only EMITS `tracing`
        # events). Pure-eval RATCHET, mirroring wasmtimeSingleHolderAssert: read rcdzc's Cargo.toml and
        # throw LOUD at eval if either creeps back into `[dependencies]`/`[dev-dependencies]`. crateClosureAssert
        # can't catch this — it tracks only FIRST-PARTY path crates, and clap/tracing-subscriber are EXTERNAL,
        # so without this guard a convenience re-add would slip through green + silently re-fatten the compiler
        # (and every crate that links it as a lib). A NEW arg/trace-sink concern belongs in `rcdzc-cli`.
        compilerPureLibraryAssert =
          let
            manifest = builtins.fromTOML
              (builtins.readFile (./. + "/${rootWorkspaceCrates.rcdzc}/Cargo.toml"));
            forbidden = [ "clap" "tracing-subscriber" ];
            deps = (manifest.dependencies or { }) // (manifest.dev-dependencies or { });
            present = builtins.filter (d: (deps.${d} or null) != null) forbidden;
          in
          if present != [ ] then
            throw ("flake.nix compiler-pure-library-assert: rcdzc (the compiler) declares forbidden host-CLI "
              + "dep(s) " + builtins.toString present + " — it must stay a PURE LIBRARY (operator 2026-08-30). "
              + "clap (arg parsing) + tracing-subscriber (the trace sink) live in the `rcdzc-cli` crate; move "
              + "the new concern there and depend on rcdzc-the-library instead.")
          else
            pkgs.runCommand "compiler-pure-library-assert" { } ''
              echo "ok: rcdzc is a pure library (no clap / tracing-subscriber)" > $out
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
            # `cdz test` runs the compiler + property-gen IN-PROCESS (standalone), so this uses the
            # seedCompilerTestRunner variant — the --no-default-features seedCompiler's `cdz test` REFUSES
            # ("no in-process test runner"). No delegation here (in-process); CDZ_COMPILE_BIN is set only as a
            # harmless no-op for shape-parity with the buildCadenzaProject (compile/delegate) path.
            nativeBuildInputs = [ seedCompilerTestRunner ];
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
        # Wall-clock cap (secs) on a single project's `cdz test`. A COMPILE-phase hang (e.g. the match->let
        # non-termination v-compiler-perf RCA'd) is otherwise UNBOUNDED inside the nix build sandbox:
        # CDZ_RUN_TIMEOUT caps the test RUN not the COMPILE, and #5963's timeout-minutes is GHA/CI-only — so a
        # hanging compile pegs a core to OOM (an observed pid ran 39min). This fail-fast cap bounds it. Set to
        # 30min = parity with the #5963 CI budget, and ~8x the heaviest legit suite closure (compiler-ml ~215s)
        # so it never false-fails a slow-but-progressing build even under fleet nix contention. Defense-in-depth
        # for ANY future compile hang (concierge seq-271 interim mitigation; permanent fix = v-cp #6000).
        cadTestTimeoutSecs = 1800;
        mkCadProjectTest = { name, dir }: pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-cad-test-${name}";
          version = "0.0.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = dir;
          };
          # `cdz test` is standalone-gated (in-process rcdzc runner) → use seedCompilerTestRunner; the
          # --no-default-features seedCompiler's `cdz test` refuses. CDZ_COMPILE_BIN kept as a harmless no-op.
          nativeBuildInputs = [ seedCompilerTestRunner ];
          CDZ_COMPILE_BIN = "${cdzCompile}/bin/cdz-compile";
          buildPhase = ''
            runHook preBuild
            set -o pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            # Run this project's @test suite, resolving the runtime from the nix store, under a wall-clock cap
            # (see cadTestTimeoutSecs) so a compile-phase hang fails-fast instead of running to OOM. A non-zero
            # `cdz test` propagates (PIPESTATUS[0] past the tee) and fails the build.
            echo "== cdz test ${name} (wall-clock cap ${toString cadTestTimeoutSecs}s) =="
            timeout --kill-after=30s ${toString cadTestTimeoutSecs} \
              cdz test "implementation/${name}" | tee "$TMPDIR/cad-test.out"
            st=''${PIPESTATUS[0]}
            if [ "$st" = 124 ] || [ "$st" = 137 ]; then
              echo "TIMEOUT: 'cdz test ${name}' exceeded the ${toString cadTestTimeoutSecs}s wall-clock cap — treating as a compile/run HANG (fail-fast, not an OOM core-peg)." >&2
              exit 1
            fi
            [ "$st" = 0 ] || exit "$st"
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

        # ── test-shred: per-@test wasm matrix (v-test-shred; design/DESIGN-test-shred-per-test-caching.md) ──
        # Mirrors the corpus per-case caching graph for a cadenza @test SUITE: SHRED a project (`cdz test
        # --emit-shred`, in-process rcdzc, wasmtime-free, content-addressed) into per-@test wasm + a manifest,
        # then one exec per @test that runs it (`cdz-run` + the value-heap store), graded by EXIT CODE (clean =
        # PASS, trap = FAIL — a @test has no expected value). Enumerated at EVAL via the compiler-informed
        # `testDiscovery` scoped-cached-IFD (`cdz test --list --format nix`, see below — NO committed index).
        # PER-SUITE MODE (v-test-shred coverage audit 2026-08-29 — see testShredSuites): --standalone emits each
        # @test independently (FULL coverage; exec runs the per-test wasm directly, `--peer`-wiring a dep if the
        # manifest records one) — the mode a small-closure suite (iterators) needs to run ALL its tests + be
        # retire-ready. --two-stage instead emits ONE shared closure fragment + per-@test fragments (compile-
        # once caching) and each exec SPLICES+COMPILES the closure + its fragment (`cdz-compile <closure> <test>
        # --export`); it trades coverage for caching (a @test whose fragment the emit_fragment path can't yet
        # lower — higher-order params / generic-open user-sum — is ABSENT from the manifest and SKIPS, exit 0),
        # so it is reserved for HEAVY suites once that coverage is real. ADDITIVE — does NOT retire
        # `cad-tests`/`testCadenzaProject`; per-suite retire as each suite's per-test shred covers ALL its tests.
        # iterators wired (standalone, 360/360); cad/compiler-ml/choreography follow after v-cadenza re-emit +
        # collision-free per-@test target filenames. Parse `testDiscovery`'s imported list → [{ stem; name }] per entry.
        # Keyed by (file-STEM, name): a @test name can repeat across a suite's files, so the stem (baseNameOf
        # file, ext stripped) disambiguates + matches the manifest's `file` field to resolve the RIGHT per-@test
        # fragment (spliced against the shared closure), so no collision even for same-named tests across files.
        # DISCOVERY — compiler-informed test enumeration via a SCOPED, CACHED IFD (operator OK'd IFD seq-168;
        # concierge greenlit scoped-cached-IFD as the discovery mechanism 2026-08-29; pure dyn-drv is R&D-blocked
        # in nix 2.34.8). A derivation runs `cdz test --list --format nix <proj>` (#5461) → $out = a SORTED, PURE,
        # importable nix list `[{name;is_property;file}...]` (verified: works in the --no-default-features
        # seedCompiler). The flake `import`s $out AT EVAL to fan out per-@test derivations — compiler-authoritative
        # (db.test_defs), NO committed index. IFD is SCOPED to THIS discovery ONLY (the global no-IFD convention
        # stays); nix caches the drv output, so eval re-reads only when the suite src changes (rotates the drv) —
        # not on every eval. Reversible to pure dyn-drv on a nix upgrade. Replaces the retired committed
        # tests-shred-index.txt. Keyed by (file-STEM, name): a @test name can repeat across a suite's files, so
        # the stem (baseNameOf file, ext stripped) disambiguates + matches the manifest's `file` field.
        testDiscovery = { proj, dir }:
          pkgs.runCommand "test-discovery-${proj}" { nativeBuildInputs = [ seedCompilerTestRunner ]; } ''
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            cdz test --list --format nix ${pkgs.lib.fileset.toSource { root = ./.; fileset = dir; }}/implementation/${proj} > "$out"
          '';
        testShredIndexEntries = proj: dir:
          map
            (e: { stem = pkgs.lib.head (pkgs.lib.splitString "." (baseNameOf e.file)); name = e.name; })
            (import (testDiscovery { inherit proj dir; }));
        # SHRED (content-addressed) — compile the project's @tests to per-@test wasm + `manifest.cdzb` ONCE.
        # Closure = the `cdz` binary (emit-shred drives rcdzc IN-PROCESS; wasmtime-free; NO store — compile
        # only). CA so a compiler change that re-emits identical wasm cache-hits every exec.
        mkTestShred = { proj, dir, mode }:
          pkgs.runCommand "test-shred-${proj}"
            {
              # cdzRun (cranelift-ON) for the standalone AOT precompile below (seq-271). two-stage doesn't
              # precompile (its per-test wasm isn't self-contained — needs the compile-time splice).
              # `cdz test --emit-shred` is standalone-gated (in-process rcdzc) → seedCompilerTestRunner, NOT the
              # --no-default-features seedCompiler (whose `cdz test` refuses). cdzRun (cranelift-ON) is added for
              # the standalone AOT precompile below (seq-271); two-stage doesn't precompile.
              nativeBuildInputs = [ seedCompilerTestRunner ] ++ pkgs.lib.optional (mode == "standalone") cdzRun;
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME" "$out"
            # emit-shred EXITS NON-ZERO when some @tests DECLINE — but it still WRITES the manifest + the
            # shreddable per-@test wasm, so tolerate the non-zero exit and guard only on a MISSING manifest (a
            # real emit failure). WHICH tests decline depends on the MODE: --standalone emits the FULL suite
            # (each @test compiled independently → NO declines for a small-closure suite like iterators → exit 0
            # → this branch is inert, all N run), whereas --two-stage trades coverage for a compile-once shared
            # closure (emit_fragment cannot yet lower higher-order params / generic-open user-sum type-decl, so
            # those @tests decline + SKIP). Per-suite mode (below) picks standalone for full-coverage/retire-ready
            # small suites and reserves two-stage for HEAVY suites once emit_fragment coverage is real.
            if ! cdz test --emit-shred --${mode} ${pkgs.lib.fileset.toSource { root = ./.; fileset = dir; }}/implementation/${proj} --out-dir "$out"; then
              [ -f "$out/manifest.cdzb" ] || { echo "test-shred: --${mode} produced no manifest for ${proj} (real emit failure)" >&2; exit 1; }
              echo "test-shred: --${mode} for ${proj} exited non-zero (expected under two-stage — some @tests declined, e.g. higher-order-param / user-sum re-emit gap); manifest present, proceeding with the shreddable subset" >&2
            fi
            # AOT (standalone, seq-271): precompile each self-contained per-test wasm → .cwasm ONCE (cranelift-ON,
            # CA-cached WITH the shred), so mkTestExec runs cranelift-FREE deserialize-only (the ~41% JIT cut).
            # Standalone tests are self-contained (manifest main-file="" → no peer) → all precompilable. The
            # `[ -e ] || continue` guard is NULLGLOB-safe (nix stdenv has nullglob ON; a bare `*.wasm` no-match
            # would otherwise misfire — the mkCorpusPrecompile lesson). manifest is .cdzb, so `*.wasm` = per-test.
            ${pkgs.lib.optionalString (mode == "standalone") ''
              for w in "$out"/*.wasm; do
                [ -e "$w" ] || continue
                cdz-run "$w" --precompile-out "''${w%.wasm}.cwasm"
              done
            ''}
          '';
        # EXEC — grade ONE @test. Closure = the COMPILER-FREE `cdz-run` + the `cdz` binary (for the `cdz
        # convert` decode) + the value-heap store. The manifest is a cadenza-ast VALUE; `cdz convert --to
        # sexpr` renders it as (multi-line, pretty-printed) s-expression, which a gawk `RS="(entry"` pass
        # tokenizes per entry (7 positional fields: name is-property file export target main-iface main-file;
        # strings quoted, bool true/false). A @test not present in the manifest (a decliner) SKIPS.
        # EXEC (TWO-STAGE) — grade ONE @test. Decode the manifest by (fileStem, name) → export/target/main-file,
        # then SPLICE+COMPILE the SHARED closure fragment (main-file, compiled once per suite in the shred) + this
        # test's fragment (target) via cdz-compile multi-input + --export → a standalone wasm, then cdz-run it
        # (exit 0 = PASS, trap = FAIL — a @test has no expected value). The manifest is a cadenza-ast VALUE; `cdz
        # convert --to sexpr` renders it, a gawk RS="(entry" pass tokenizes the 7 positional fields (name
        # is-property file export target main-iface main-file; main-iface is "" for two-stage, main-file is the
        # closure fragment). A @test absent from the manifest (a decliner, e.g. user-sum re-emit gap) SKIPS.
        mkTestExec = { proj, shred, fileStem, testName, mode }:
          pkgs.runCommand "test-shred-exec-${proj}-${fileStem}-${testName}"
            {
              # two-stage needs cdz-compile for the closure+fragment SPLICE; standalone runs the per-test wasm
              # DIRECTLY (leaner closure — the point of standalone for a small suite), so it drops cdz-compile.
              nativeBuildInputs = [ seedCompiler cdzRun pkgs.gawk ]
                ++ pkgs.lib.optional (mode == "two-stage") cdzCompile;
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            # (CDZ_STORE set per-branch below — AOT uses componentStoreCwasm, JIT/two-stage use componentStore.)
            # Decode this @test's manifest entry by (name, fileStem). 7 positional fields:
            # name is-property file export target main-iface main-file. main-iface/main-file mean different
            # things per mode: STANDALONE — main-iface is the peer interface + main-file its wasm (a --peer
            # dep, both "" for a self-contained test); TWO-STAGE — main-file is the shared closure fragment to
            # splice (main-iface ""). We print all four (export target iface main) and branch on mode below.
            line=$(cdz convert --from binary --to sexpr ${shred}/manifest.cdzb \
              | gawk -v n="${testName}" -v fs="${fileStem}" 'BEGIN { RS = "\\(entry" } NR > 1 {
                  c = 0; nf = split($0, t, /[ \t\n]+/);
                  for (i = 1; i <= nf; i++) { if (t[i] != "") { c++; f[c] = t[i]; if (c == 7) break } }
                  for (k = 1; k <= 7; k++) { gsub(/^"/, "", f[k]); gsub(/"?\)+$/, "", f[k]); gsub(/"$/, "", f[k]) }
                  if (f[1] == n && f[3] == fs) { print f[4] "\t" f[5] "\t" f[6] "\t" f[7] }
                }')
            if [ -z "$line" ]; then
              # Under STANDALONE the full suite emits, so a missing entry is unexpected (loud); under TWO-STAGE a
              # decliner (higher-order param / user-sum re-emit gap) legitimately SKIPS (testCadenzaProject covers it).
              ${if mode == "standalone" then ''
                echo "test-shred exec: @test ${fileStem}::${testName} MISSING from the standalone manifest — standalone emits the full suite, so this is a real emit/discovery drift, not a decline" >&2
                exit 1
              '' else ''
                echo "skip: @test ${fileStem}::${testName} not in shred manifest (declined — two-stage emit_fragment gap)" > "$out"
                exit 0
              ''}
            fi
            IFS=$'\t' read -r texport ttarget tiface tmain <<< "$line"
            ${if mode == "standalone" then ''
              # STANDALONE: AOT/JIT split (mirrors mkCorpusExec, seq-271). A non-peer test (main-file="") with a
              # precompiled cwasm (from the shred) runs CRANELIFT-FREE via deserialize (the ~41% JIT cut);
              # componentStoreCwasm resolves store deps as <hash>.cwasm (#5922), runtimeDebugCwasm is the
              # precompiled debug runtime. A --peer test (not precompiled-capable) or a missing cwasm JIT-falls-
              # back to cdzRun — byte-for-byte the pre-AOT behavior.
              cwasm="${shred}/''${ttarget%.wasm}.cwasm"
              if [ -z "$tmain" ] && [ -e "$cwasm" ]; then
                export CDZ_STORE="${componentStoreCwasm}"
                ${cdzRunExec}/bin/cdz-run "$cwasm" --precompiled --runtime ${runtimeDebugCwasm} --call "$texport"
              else
                export CDZ_STORE="${componentStore}"
                args=("${shred}/$ttarget" --call "$texport" --runtime ${runtimeDebug})
                if [ -n "$tmain" ]; then args+=(--peer "$tiface=${shred}/$tmain"); fi
                cdz-run "''${args[@]}"
              fi
            '' else ''
              # TWO-STAGE: SPLICE+COMPILE the shared closure fragment (built once, cache-HIT across the suite) +
              # this test's fragment → standalone wasm (cdz-compile multi-input + --export), then run it.
              cdz-compile ${shred}/"$tmain" ${shred}/"$ttarget" --export "$texport" -o test.wasm
              cdz-run test.wasm --call "$texport" --store "${componentStore}" --runtime ${runtimeDebug}
            ''}
            echo "ok: @test ${proj}/${testName} PASS" > "$out"
          '';
        # Per-suite check MAP `{ "<name>" = execDrv }` — shred once, one exec per @test (parallel, CA-cached).
        testShredSuiteChecks = { proj, dir, mode }:
          let shred = mkTestShred { inherit proj dir mode; };
          in builtins.listToAttrs (map
            (e: { name = "${e.stem}::${e.name}"; value = mkTestExec { inherit proj shred mode; fileStem = e.stem; testName = e.name; }; })
            (testShredIndexEntries proj dir));
        # Per-suite AGGREGATE — every @test's exec marker (under standalone every case RUNS; under two-stage a
        # decliner is exit 0 = skip). Non-vacuity guarded.
        mkTestShredSuiteAgg = { proj, dir, mode }:
          let cases = testShredSuiteChecks { inherit proj dir mode; };
          in assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "test-shred-${proj}-all" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: test-shred ${proj} — ${toString (builtins.length (builtins.attrNames cases))} @tests via shred→per-test exec (decliners skip)" > "$out"
          '';
        # (The committed-index DRIFT-GUARD was removed: it jq-parsed `cdz test --list` as JSON, which #5360
        # flipped to cadenza-ast-binary → the guard reds. And it guarded the committed-index approach the
        # operator directed us OFF of — the committed tests-shred-index.txt is now REMOVED (#5473 wired the
        # compiler-informed `testDiscovery` scoped-cached-IFD e2e-green, so `testShredIndexEntries` imports
        # discovery, not the file). So the guard is both broken by #5360 AND obsolete; dropped.)
        # v1 SUITES — each suite picks its shred MODE (v-test-shred coverage audit 2026-08-29):
        #   · standalone = FULL coverage (every @test emitted + compiled independently); the mode for a
        #     small-closure suite (iterators) that must run ALL its tests to be retire-ready (drop its coarse
        #     cad-tests arm). iterators is 360/360 under standalone — two-stage only ran 56/360 (the other 304
        #     declined the emit_fragment higher-order-param / generic-user-sum gap and SKIPPED, a HOLLOW green),
        #     and iterators' closure is small enough that two-stage's compile-once caching buys nothing.
        #   · two-stage = compile-once shared closure (trades coverage for caching); correct ONLY for a HEAVY
        #     suite (e.g. compiler-ml, ~215s closure) AND only once emit_fragment coverage is real (currently
        #     64/854 for compiler-ml) — so NO suite wires two-stage yet; it stays defined for when that lands.
        # cad/choreography follow (choreography needs unique per-@test target filenames for its ~3 cross-file
        # same-name @tests before its flat layout is collision-free; both await v-cadenza re-emit coverage).
        testShredSuites = {
          iterators = { dir = ./implementation/iterators; mode = "standalone"; };
          # seq-271 (operator: compiler-ml now optional → shred + AOT-cache ALL other pkgs). cad (138/138),
          # choreography (177/177) + music (299/299) are FULL standalone (v-test-shred validated fresh on
          # current main) → they auto-inherit the #5970 AOT wiring (per-test .cwasm precompile in mkTestShred +
          # cranelift-free mkTestExec) = the caching win. (des: ~0 @tests, skip; compiler-ml: opt-out, declines.)
          cad = { dir = ./implementation/cad; mode = "standalone"; };
          choreography = { dir = ./implementation/choreography; mode = "standalone"; };
          music = { dir = ./implementation/music; mode = "standalone"; };
        };
        testShredFileAggs = pkgs.lib.mapAttrs'
          (proj: cfg: pkgs.lib.nameValuePair "test-shred-${proj}"
            (mkTestShredSuiteAgg { inherit proj; inherit (cfg) dir mode; }))
          testShredSuites;

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
        # The cadenza-syntax crate + its extracted surface sub-crates (#5076/#5082: core is the dep-light
        # bottom; cedar/json/sexpr/toml are the per-surface splits). Enumerated ONCE and spliced into the
        # hand-listed wasm/guide filesets below, which snapshot cadenza-syntax's transitive crate dirs for
        # the STANDALONE rcdzc-wasm / cdz-wasm workspaces (their own leaf locks — NOT covered by
        # rootWorkspaceCrates' crane machinery, so the closure walk can't feed them). A future surface-crate
        # add now updates THIS one list instead of three filesets — the durable fix for the crate-split
        # whack-a-mole (v-fleet-tooling flag: fast-gate #5056, then this #5076 gap in 3 more filesets).
        cadenzaSyntaxCrateDirs = [
          "cadenza-syntax" "cadenza-syntax-core" "cadenza-syntax-cedar"
          "cadenza-syntax-json" "cadenza-syntax-sexpr" "cadenza-syntax-toml"
        ];
        # The FULL crate-dir list each standalone wasm workspace snapshots into its src fileset. Named
        # (not inlined) so the DRIFT-ASSERT below can prove the list covers every LOCAL crate the workspace
        # actually resolves. rcdzc-wasm builds the compiler-as-wasm; cdz-wasm builds the browser compiler
        # (guide examples). Both carry the same closure minus/plus their own crate. `cdz-run cdz-rt cdz-num`
        # are kept as a harmless superset (leftover from rcdzc's pre-#5000 dev-deps — unused-but-present
        # source never breaks a build; the assert only requires COVERAGE, not exactness).
        rcdzcWasmCrateDirs = [ "rcdzc-wasm" "rcdzc" "cadenza-ast" "cadenza-compile-abi" "cdz-run" "cdz-rt" "cdz-num" ] ++ cadenzaSyntaxCrateDirs;
        cdzWasmCrateDirs = [ "cdz-wasm" "rcdzc" "cadenza-ast" "cadenza-compile-abi" "cdz-run" "cdz-rt" "cdz-num" ] ++ cadenzaSyntaxCrateDirs;
        # DURABLE DRIFT-GUARD (v-fleet-tooling +1, 2026-08-28): the standalone rcdzc-wasm / cdz-wasm
        # workspaces have their OWN leaf Cargo.lock and are NOT covered by rootWorkspaceCrates' crane
        # closure machinery, so a crate split/add that changes what they resolve silently omits the new
        # crate dir from the hand-listed src fileset → `--locked` build fails fleet-wide (the recurring
        # class: fast-gate dirCaseArms #5056, then the #5076 source-filter gap in 3 filesets). This asserts,
        # at EVAL, that every LOCAL (path, no `source =`) crate in each leaf lock has its dir present in that
        # workspace's crate-dir list — so a future split that adds a local crate to the lock throws LOUD
        # here instead of red-building later. Mirrors crateClosureAssert's fail-fast discipline for the
        # workspaces the root closure-walk can't see. (Coverage, not equality — a superset dir list is fine.)
        leafLockLocalCrates = lockPath:
          let lock = builtins.fromTOML (builtins.readFile lockPath);
          in map (p: p.name) (builtins.filter (p: !(p ? source)) (lock.package or [ ]));
        standaloneWasmWorkspaceAssert =
          let
            checks = [
              { name = "rcdzc-wasm"; lock = ./implementation/seed/crates/rcdzc-wasm/Cargo.lock; dirs = rcdzcWasmCrateDirs; }
              { name = "cdz-wasm"; lock = ./implementation/seed/crates/cdz-wasm/Cargo.lock; dirs = cdzWasmCrateDirs; }
            ];
            missingFor = c: builtins.filter (n: !(builtins.elem n c.dirs)) (leafLockLocalCrates c.lock);
            offenders = builtins.filter (c: (missingFor c) != [ ]) checks;
          in
          if offenders != [ ] then
            throw ("flake.nix standalone-wasm-workspace drift-assert: a leaf Cargo.lock resolves LOCAL crates "
              + "absent from its src fileset crate-dir list — the wasm/guide `--locked` build will fail. "
              + "Add the missing dir(s) to the corresponding *CrateDirs list. "
              + builtins.concatStringsSep " | " (map (c: c.name + " missing [" + builtins.concatStringsSep " " (missingFor c) + "]") offenders))
          else
            pkgs.runCommand "standalone-wasm-workspace-assert" { } ''
              echo "ok: rcdzc-wasm + cdz-wasm src filesets cover every local crate in their leaf locks" > $out
            '';
        rcdzcWasmSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions (map (c: ./implementation/seed/crates + ("/" + c))
            rcdzcWasmCrateDirs ++ [ ./rust-toolchain.toml ]);
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

        # cdz-wasm NATIVE test + clippy + fmt (v-cdz-crate-split 2026-08-30) — the browser compiler's own
        # test suite, run on the HOST (not wasm32). cdz-wasm is a STANDALONE workspace (its own Cargo.lock,
        # excluded from the root), so it is NOT covered by rootWorkspaceCrates' per-crate test-crane/clippy
        # shards — its consumers went UNGATED, which is exactly how the binary-AST wire flips silently broke
        # its `type_at`/`define_at`/`disposition`/`export_types` `from_utf8`-on-binary consumers (#6324 +
        # #6342): the cdz LSP decoders were gate-covered but the cdz-wasm BROWSER consumers were not. This
        # check closes that hole (concierge follow-up). Mirrors `rcdzcWasmNativeCheck`, reusing cdz-wasm's OWN
        # vendor + closure src. It runs the NATIVE tests (host target), so it does NOT hit the wasm-execution
        # binaryen OOB that keeps `guideExamplesCheck` advisory — so unlike that check it is MERGE-GATED
        # (localGate below). The wasm32 build half is `cdzWasmPkg`; together they cover the crate.
        cdzWasmNativeCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-wasm-native";
          version = "0.0.0";
          src = guideCompilerWasmSrc;
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            ${mkCargoVendorEnv { vendor = cdzWasmVendor; }}
            cd implementation/seed/crates/cdz-wasm
            cargo test --locked
            cargo clippy --all-targets --locked -- -D warnings
            cargo fmt --check
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-wasm native (test + clippy + fmt)" > "$out"
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
        # seq-273 (operator, co-land with v-runtime slice-1): cdz-runtime NO LONGER `#[path]`-source-includes
        # cadenza-ast — it declares a real `cadenza-ast = { path = "../cadenza-ast", default-features = false }`
        # crate dep (its no_std+alloc CORE). The #459 cross-crate-LTO/frozen-hash worry was RESOLVED host-local:
        # the runtime hash is injected PER-HOST from the same nix closure (runtime_abi.rs:95-96, no cross-host
        # repro requirement), and lto+cu=1 still erases the crate boundary → a one-time hash re-record (v-runtime
        # verified 05L2JC reproducible on aarch64; v-nix confirmed the semantics + this flake side). So the nix
        # runtime source closure now STAGES THE cadenza-ast CRATE (src + Cargo.toml) so cargo/cargo-component
        # resolves the path-dep, INSTEAD of deriving+staging the (now-gone) `#[path]` sibling files. A cadenza-ast
        # change rotates the runtime (bytes depend) — correct + expected.
        runtimeSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions ([
            ./implementation/seed/crates/cdz-runtime
            # cadenza-ast is now a real path-dep of cdz-runtime (seq-273) — stage the whole crate so cargo
            # resolves it (its no_std+alloc core via default-features=false). Replaces the old `#[path]`-derived
            # codec-core staging (runtimePathIncludes). The shared canonical binary codec (ast value model +
            # Builder + codec::encode/decode) still makes the runtime's ast-encode/ast-decode heap ops
            # byte-identical to the compile-time `Ast.encode` fold — now via the crate dep, not source-include.
            ./implementation/seed/crates/cadenza-ast
          ]
          ++ [
            # The runtime's world imports `cadenza:nfc/normalize` (FINDING#23); its Cargo.toml points
            # cargo-component's WIT resolution at the sibling NFC crate's WIT. Scope to just the WIT (all the
            # runtime build reads) so a cdz-nfc src change doesn't rotate the runtime.
            ./implementation/seed/crates/cdz-nfc/wit
            ./rust-toolchain.toml
          ]);
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
        mkStripComponent = { pname, crateDir, artifact, src, vendor, features ? [ ], emitRaw ? false, stampNfcHash ? null, world ? null }:
          pkgs.stdenvNoCC.mkDerivation {
            inherit pname src;
            version = "0.0.0";
            outputs = if emitRaw then [ "out" "raw" ] else [ "out" ];
            # CONTENT-ADDRESSED (v-nix 2026-08-30, concierge-priority herd-eliminator): the runtime/nfc
            # component's OUTPUT is the canonicalized wasm bytes (cdz content-hash 058B5h/…), which are
            # STABLE across unrelated main commits — but as an input-addressed derivation its nix STORE PATH
            # moved per-commit (its .drv inputs churn), so cache-warm's push at commit A was a 404 for a
            # consumer on commit B → every resuming/gate-local agent COLD-BUILT the runtime (the 43-agent
            # load-80 herd). Making it __contentAddressed makes the store path a function of the emitted
            # BYTES (not inputs), so it is STABLE across commits → cachix hits ANY commit → no cold-build
            # herd. This does NOT change REQUIRED_RUNTIME_HASH (that is cdz's content-hash of the same bytes,
            # unaffected by the nix store path) → NO flag-day. Consistent with the flake's existing CA
            # derivations (test-shred/corpus/guideShred/cwasm), consumed build-time so no IFD.
            __contentAddressed = true;
            outputHashMode = "recursive";
            outputHashAlgo = "sha256";

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
              ${pkgs.lib.optionalString (world != null) ''
                # RC-TRACE variant (v-nix 2026-08-31): retarget cargo-component at a non-default world for
                # THIS build only. cargo-component reads the world from [package.metadata.component.target]
                # world = "…"; patch it in-place (fileset copies are read-only → chmod first) + chmod the
                # checked-in src/bindings.rs so cargo-component can regenerate it for the new world (it is a
                # DO-NOT-EDIT wit-bindgen file overwritten per targeted world). Only the rc-trace runtime
                # variant passes world = "runtime-debug" (heap + debug-trace export); release + the
                # debug-counters leak-check runtime pass no `world` and keep the committed `world runtime`,
                # so REQUIRED_RUNTIME_HASH + DEBUG_RUNTIME_HASH are unaffected. Sandbox-only mutation (the
                # throwaway fileset copy) — NOT the dev tree, so no SIGKILL-orphan hazard (option B moved the
                # export off `debug-counters` onto `rc-trace-export`, so xtask codegen never patches a world).
                chmod u+w Cargo.toml src/bindings.rs
                sed -i 's/^world = "runtime"/world = "${world}"/' Cargo.toml
                grep -q 'world = "${world}"' Cargo.toml || { echo "rc-trace world patch FAILED: world line not rewritten" >&2; exit 1; }
              ''}
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
        mkRuntime = { pname, features, emitRaw ? false, world ? null }:
          mkStripComponent {
            inherit pname features emitRaw world;
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

        # The DEBUG-COUNTERS runtime — same code + the `live-objects` leak counter + (2026-08-31) the
        # rc-trace INSTRUMENTATION (`--features debug-counters`); the Perceus leak-check harness composes it
        # (DEBUG_RUNTIME_HASH). world `runtime` (NO debug-trace export — that is gated behind the separate
        # `rc-trace-export` feature, absent here), so this build stays world-`runtime`-only and its hash
        # re-bakes ONLY for the new instrumentation bytes (05mPZx-successor), not for any export.
        runtimeDebug = mkRuntime {
          pname = "cdz-runtime-component-debug";
          features = [ "debug-counters" ];
        };

        # The RC-TRACE runtime variant (v-nix + v-runtime 2026-08-31, option B) — the debug-counters runtime
        # PLUS the debug-trace drain export (`--features debug-counters,rc-trace-export`), targeting `world
        # runtime-debug` (heap + debug-trace). FLAKE-ONLY: nothing pins its hash (rc-trace is a diagnostic,
        # not a gate assertion), so cdz-run --rc-trace consumes it by EXPLICIT --runtime <this path>, exactly
        # like the leak-check AOT execs pass --runtime runtimeDebugCwasm. Isolating the export to this variant
        # (feature `rc-trace-export` gates guest.rs; `world` retargets cargo-component) keeps the release
        # 058B5h AND the DEBUG_RUNTIME_HASH leak-check runtime byte-identical to their no-export builds — no
        # re-pin, and xtask codegen (which builds only the plain debug-counters world-`runtime` runtime) never
        # touches a non-default world (no E0433).
        runtimeRctrace = mkRuntime {
          pname = "cdz-runtime-component-rctrace";
          features = [ "debug-counters" "rc-trace-export" ];
          world = "runtime-debug";
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
          # SCOPED (v-nix 2026-08-28): build cdz-world-artifact from its own closure, NOT the broad
          # platformItestSrc — else an xtask/corpus/unrelated edit rotates this, and it feeds the runtime
          # WIT world → runtime component → hash → seedCompiler. `extra` keeps the platform WIT the
          # installPhase reads (world.wit + wit/test/arg-probe.wit).
          src = scopedToolSrc {
            crate = "cdz-world-artifact";
            extra = [ ./implementation/seed/crates/cdz-platform/wit ];
          };
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            chmod -R u+w .
            ${stubNonClosure (crateClosure "cdz-world-artifact")}
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
            # OVERLAY (v-nix, codegen→build-time-nix): stage the build-time-generated contract schemas over
            # cdz-platform/src/contracts — this bin compiles cdz-platform's real src. Byte-identical no-op
            # today; load-bearing after the flip drops the committed src/contracts. (Same overlay the per-crate
            # clippy/test-cdz-platform checks apply via craneCrateCommon.)
            chmod -R u+w implementation/seed/crates/cdz-platform/src
            mkdir -p implementation/seed/crates/cdz-platform/src/contracts
            cp ${cdzPlatformContracts}/contracts/*.rs implementation/seed/crates/cdz-platform/src/contracts/
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
          # SCOPED (v-nix 2026-08-28): cdz-contract's own closure, not the broad platformItestSrc.
          src = scopedToolSrc { crate = "cdz-contract"; };
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            chmod -R u+w .
            ${stubNonClosure (crateClosure "cdz-contract")}
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
        # SCOPED src (v-nix 2026-08-28): cdzComponentRewrite is on the RUNTIME critical path — mkRuntime
        # stamps the value-heap component with it, so its hash feeds runtime/nfc-hash → seedCompiler. It
        # MUST NOT use the broad platformItestSrc (which unions ./xtask + ALL seed crates + compiler-ml +
        # spec/semantics): that made ANY edit under those paths (xtask, a corpus .sexp, an unrelated seed
        # crate — the fleet touches them every few min) spuriously rotate this tool → rebuild the runtime
        # component + hashes + the whole compiler world (an incremental-cache regression; also exposed a
        # latent OOB via non-reproducible rebuild — v-xtask-decompose 87ba0546). Scope it to just the
        # cdz-component-rewrite crate + its dep-closure src + workspace-parse manifests, exactly like
        # seedCompilerSrc / the crane per-crate checks, with synthetic stubs for non-closure members so
        # `cargo -p cdz-component-rewrite` parses the workspace without their real src.
        cdzComponentRewrite = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-component-rewrite";
          version = "0.0.0";
          src = scopedToolSrc { crate = "cdz-component-rewrite"; };
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            chmod -R u+w .
            ${stubNonClosure (crateClosure "cdz-component-rewrite")}
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
        mkPhaseBin = { pname, crate, bin ? pname, closure, injectRuntimeHash ? false, extraArgs ? "" }:
          craneLib.buildPackage {
            inherit pname;
            version = "0.0.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions (
                (pkgs.lib.concatMap crateCompileSrc closure)
                ++ nonClosureManifests closure
                ++ [ ./xtask/Cargo.toml ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]);
            };
            # WARM RELEASE DEP-CACHE (v-nix, operator 2026-08-29 "stop recompiling wasmtime over and over"):
            # consume the shared `cargoArtifactsRelease` deps layer so crane RESTORES the pre-compiled
            # dependency closure (wasmtime/cranelift are ~the bulk, and cdz-run/cdz-rust-run pull them in)
            # instead of recompiling it from scratch. Previously this was a RAW `pkgs.stdenvNoCC` +
            # `cargo build --release` with NO dep-cache, so every scoped-src rotation — any cdz-run edit
            # (grade.rs/cli.rs/main.rs, edited constantly) or any Cargo.lock churn (the ongoing crate splits)
            # — recompiled the ENTIRE closure INCLUDING wasmtime, fleet-wide, on the critical path of every
            # corpus/guide exec (cdz-run feeds them all). crane restores the release deps target/ (matching
            # profile — cargoArtifactsRelease is CARGO_PROFILE=release), then builds only first-party. Mirrors
            # `seedCompiler`'s crane shape (scoped src + stubNonClosure + seedCargoVendor + hash inject).
            cargoArtifacts = cargoArtifactsRelease;
            cargoVendorDir = seedCargoVendor;
            # preBuild (crane's hook — runs AFTER crane restores cargoArtifactsRelease' target/, before build).
            preBuild = ''
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
              chmod -R u+w .
              ${stubNonClosure closure}
              [ -f xtask/src/main.rs ] || { mkdir -p xtask/src; echo "fn main(){}" > xtask/src/main.rs; }
              [ -f xtask/src/lib.rs ] || echo "" > xtask/src/lib.rs
            '';
            # crane injects --locked + --release; scope to just this phase bin (its equivalent of the raw
            # `cargo build -p <crate> --bin <bin>`). Build only — no tests (the gate/CI run those).
            cargoExtraArgs = "-p ${crate} --bin ${bin} ${extraArgs}";
            doCheck = false;
          };
        # shred (parser closure — excludes rcdzc), build (compiler closure = rcdzc), exec (runtime closure —
        # cdz-run deps wasmtime/cadenza-syntax/cdz-contract/cdz-rt, NO rcdzc, so COMPILER-FREE by construction).
        cdzCorpus = mkPhaseBin { pname = "cdz-corpus"; crate = "cdz-corpus"; closure = crateClosure "cdz-corpus"; };
        # The `cdz-compile` bin now lives in the `rcdzc-cli` crate (the clap arg-parsing layer), NOT in
        # the `rcdzc` compiler LIBRARY — operator directive 2026-08-30 made rcdzc a PURE library (no clap).
        # rcdzc-cli's closure = rcdzc's closure ∪ {rcdzc-cli} (clap/tracing-subscriber are external, not
        # first-party), so the compiler-only closure is unchanged apart from the thin clap leaf.
        cdzCompile = mkPhaseBin { pname = "cdz-compile"; crate = "rcdzc-cli"; bin = "cdz-compile"; closure = crateClosure "rcdzc-cli"; injectRuntimeHash = true; };
        cdzRun = mkPhaseBin { pname = "cdz-run"; crate = "cdz-run"; closure = crateClosure "cdz-run"; };
        # cdzRunExec — CRANELIFT-FREE corpus executor (seq-250/271 AOT split, #5893/#5910/#5922). Drops the
        # default `cranelift` feature → deserialize-only (Component::deserialize of precompiled .cwasm), no JIT.
        cdzRunExec = mkPhaseBin { pname = "cdz-run-exec"; crate = "cdz-run"; bin = "cdz-run"; closure = crateClosure "cdz-run"; extraArgs = "--no-default-features"; };
        # cdzHandWrapper / cdzRunHandWrapper — the SELF-CONTAINED front-end wrappers (hoisted out of apps.cdz /
        # apps.cdz-run so `apps.build` can materialize the SAME wrapper into a worktree's target/release/ — no
        # drift). Each exports the phase-bin overrides (CDZ_COMPILE_BIN / CDZ_STORE / CDZ_RUN_BIN / CDZ_CALC_BIN,
        # caller-override-honored via :-) then execs the seed bin, so a hand-run `./target/release/cdz compile`
        # uses the warm nix compiler + store instead of shelling to nix/cargo (the raw seed bin's fallback fails
        # outside a flake + is slow inside one). componentStore/cdzCompile/cdzCalc are later in this let — fine,
        # the let is recursive. This is why apps.build's materialized bins "just work" for the full compile→run
        # loop with no per-worktree cargo.
        cdzHandWrapper = pkgs.writeShellApplication {
          name = "cdz";
          runtimeInputs = [ ];
          text = ''
            export CDZ_COMPILE_BIN="''${CDZ_COMPILE_BIN:-${cdzCompile}/bin/cdz-compile}"
            export CDZ_STORE="''${CDZ_STORE:-${componentStore}}"
            export CDZ_RUN_BIN="''${CDZ_RUN_BIN:-${cdzRun}/bin/cdz-run}"
            export CDZ_CALC_BIN="''${CDZ_CALC_BIN:-${cdzCalc}/bin/cdz-calc}"
            # CDZ_RUST_RLIB_DIR (v-cdz-crate-split #5689): `cdz run-rust` links the emitted rust driver against
            # the prebuilt cdz-rt/cdz-num/cadenza-ast rlibs. The nix cdz bin ships NO rlibs beside the exe (so
            # the exe-relative fallback found nothing → E0433, killing cdz-smith's rust oracle + breaker's rust
            # differential). Point it at `rustRlibs` (the same prebuilt rlib dir the corpus-rust exec grader
            # uses via --cdz-rt-dir). Caller override honored via :- (a cargo build sets its own / leaves unset).
            export CDZ_RUST_RLIB_DIR="''${CDZ_RUST_RLIB_DIR:-${rustRlibs}}"
            exec "${seedCompiler}/bin/cdz" "$@"
          '';
        };
        cdzRunHandWrapper = pkgs.writeShellApplication {
          name = "cdz-run";
          runtimeInputs = [ ];
          text = ''
            export CDZ_STORE="''${CDZ_STORE:-${componentStore}}"
            exec "${seedCompiler}/bin/cdz-run" "$@"
          '';
        };
        # cdz-calc: the standalone calc/repl binary `cdz calc` (alias `cdz repl`) forwards to (v-cdz-crate-split
        # #5167 dropped cdz's cdz-calc lib dep — it pulled cdz-run/wasmtime transitively — making it a
        # CDZ_CALC_BIN passthrough). Built here so apps.cdz can inject CDZ_CALC_BIN for an interactive
        # `cdz calc`/`cdz repl` in the nix shell / packaged cdz (no gate site runs the REPL, so no gate needs it).
        cdzCalc = mkPhaseBin { pname = "cdz-calc"; crate = "cdz-calc"; closure = crateClosure "cdz-calc"; };
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
          # CDZ_RUN_BIN (v-cdz-crate-split, operator wasmtime-out-of-the-compiler): PRE-WIRED INERT. Today the
          # --no-default-features seedCompiler runs `cdz run` (the descriptor() exec) IN-PROCESS and ignores
          # CDZ_RUN_BIN, so this is a NO-OP; when their cdz-side forward lands (cdz run -> spawn external
          # cdz-run, like cdz compile -> cdz-compile), this env makes the run reach the cdzRun binary WITHOUT
          # a flake-day. Points at cdzRun, the same content-addressed run binary mkCorpusExec uses. Resolution
          # is $CDZ_RUN_BIN -> sibling -> $PATH (locate_plugin), so the explicit path wins — no PATH change.
          export CDZ_RUN_BIN="${cdzRun}/bin/cdz-run"
          # Stage the lib under its CLEAN name: `cdz compile` derives a package-file's module name from the
          # input's FILE STEM, and the import is `from "contract-id"`, so the input must be named
          # `contract-id.cdz`. The raw store path is `<hash>-contract-id.cdz` (stem `<hash>-contract-id`) →
          # CDZ0201 "unknown package file `contract-id`". Copy to a clean-named temp file and pass that.
          cp ${./implementation/seed/crates/cdz-platform/guests/contract-id.cdz} "$TMPDIR/contract-id.cdz"
          cdz-contract hash ${contractSourcesDir} \
            --lib "$TMPDIR/contract-id.cdz" \
            --cdz ${seedCompiler}/bin/cdz --out "$out"
        '';

        # cdzPlatformContracts (v-nix, operator codegen→build-time-nix): run v-xtask-decompose's standalone
        # xtask-codegen-contracts bin (#5209) to EMIT cdz-platform/src/contracts/*.rs at BUILD time, so the
        # generated contract schemas need not be committed source. The bin reads CDZ_REPO_ROOT-relative:
        # implementation/seed/crates/cdz-platform/contracts/{kernel,userspace}/*.cdz + guests/contract-id.cdz,
        # runs `cdz convert`→binary AST + executes descriptor() for the id (same delegate-cdz shape as
        # contractHashes above: CDZ_STORE + CDZ_COMPILE_BIN + CDZ_RUN_BIN), then render_schema → <name>.rs +
        # mod.rs into the out dir. Stage a WRITABLE repo tree ($TMPDIR/repo) with just those inputs (the bin
        # writes a target/codegen-contract-stage scratch under CDZ_REPO_ROOT, so it must be writable). This is
        # the FIRST half of the flip (additive — nothing consumes it yet); cdzPlatformContractsMatch below
        # guards it byte-identical to the committed src/contracts until the atomic overlay-flip lands.
        cdzPlatformContracts = pkgs.runCommand "cdz-platform-contracts"
          # rustToolchain provides `rustfmt` — the bin renders via prettyplease THEN runs `rustfmt --edition
          # 2024`, falling back to raw (unformatted) prettyplease if rustfmt is absent. Without rustfmt on PATH
          # the output diverges from the committed (cargo-fmt'd, edition 2024) src/contracts on line-wrapping.
          { nativeBuildInputs = [ xtaskCodegenContractsBin seedCompiler rustToolchain ]; } ''
          set -euo pipefail
          export HOME="$TMPDIR/home"; mkdir -p "$HOME"
          root="$TMPDIR/repo"; plat="$root/implementation/seed/crates/cdz-platform"
          mkdir -p "$plat/guests"
          cp -r ${./implementation/seed/crates/cdz-platform/contracts} "$plat/contracts"
          cp ${./implementation/seed/crates/cdz-platform/guests/contract-id.cdz} "$plat/guests/contract-id.cdz"
          chmod -R u+w "$root"
          export CDZ_REPO_ROOT="$root"
          export CDZ_SEED_BIN_DIR="${seedCompiler}/bin"
          export CDZ_STORE="${componentStore}"
          export CDZ_COMPILE_BIN="${cdzCompile}/bin/cdz-compile"
          export CDZ_RUN_BIN="${cdzRun}/bin/cdz-run"
          mkdir -p "$out/contracts"
          xtask-codegen-contracts "$out/contracts"
        '';

        # wasmAbiSexpSrc: the AUTHORITATIVE hand-authored wasm-abi.sexp, staged at its repo-relative path so the
        # bin's CDZ_REPO_ROOT join resolves it. Scoped to the ONE file → cdzWasmAbi/oracle rotate only when the
        # sexp changes. It lives at the TOP-LEVEL `data/` (OUTSIDE the rust compiler tree — language-independent,
        # operator seq-173, moved there in #5333); the bin joins CDZ_REPO_ROOT with `data/wasm-abi.sexp`.
        wasmAbiSexpSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = ./data/wasm-abi.sexp;
        };
        # declinesSexpSrc (v-deferral-declines seq-106) — the DeclineId source-of-truth the codegen reads.
        declinesSexpSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = ./data/unsupported.sexp;
        };
        # cdzWasmAbi (v-nix, operator codegen→build-time-nix): the 2nd generated file — run v-xtask-decompose's
        # xtask-codegen-wasm-abi bin to EMIT rcdzc/src/backend/wasm/wasm_abi.rs at build time. FLIPPED to the
        # operator's SEXPR → RUST direction (--from-sexpr, #5316): the bin reads the authoritative wasm-abi.sexp,
        # `cdz convert`s it to cadenza-ast binary, walks + renders — so this now needs `cdz` (CDZ_SEED_BIN_DIR =
        # seedCompiler) + the sexp (CDZ_REPO_ROOT = wasmAbiSexpSrc), plus rustfmt (it renders prettyplease then
        # `rustfmt --edition 2024`). Output stays BYTE-IDENTICAL to the retired wasm-encoder default (v-xtask
        # holds that as the acceptance test), so cdzWasmAbiMatch stays green. seedCompiler is already a localGate
        # closure dep (gate-check/cad-test) → no new heavy build, just this small render consuming the warm cdz.
        cdzWasmAbi = pkgs.runCommand "cdz-wasm-abi"
          { nativeBuildInputs = [ xtaskCodegenWasmAbiBin seedCompiler rustToolchain ]; } ''
          set -euo pipefail
          mkdir -p "$out"
          export CDZ_REPO_ROOT="${wasmAbiSexpSrc}"
          export CDZ_SEED_BIN_DIR="${seedCompiler}/bin"
          xtask-codegen-wasm-abi --from-sexpr "$out/wasm_abi.rs"
        '';
        # DRIFT-GUARD: build-time-generated wasm_abi.rs MUST be byte-identical to the committed one, until the
        # atomic overlay-flip drops the committed copy (v-xtask-decompose verified #5316 --from-sexpr emits
        # diff-clean — byte-identical to the committed cargo-fmt'd file).
        cdzWasmAbiMatch = pkgs.runCommand "cdz-wasm-abi-match" { } ''
          set -euo pipefail
          if diff ${cdzWasmAbi}/wasm_abi.rs ${./implementation/seed/crates/rcdzc/src/backend/wasm/wasm_abi.rs} > wasmabi.diff; then
            echo "ok: cdzWasmAbi (build-time --from-sexpr codegen) == committed rcdzc/.../wasm_abi.rs (byte-identical)" > "$out"
          else
            echo "DRIFT: build-time wasm-abi codegen != committed wasm_abi.rs — regen committed or fix the sexpr:"
            cat wasmabi.diff; exit 1
          fi
        '';
        # cdzDeclines (v-deferral-declines seq-106; v-nix flake reg) — run xtask-codegen-declines to EMIT
        # rcdzc/src/diag/declines_generated.rs from data/unsupported.sexp at build time. Mirrors cdzWasmAbi:
        # CDZ_REPO_ROOT = the scoped sexp src, CDZ_SEED_BIN_DIR = seedCompiler (the bin reads the sexp as
        # binary AST via `cdz convert`), first arg = the output path.
        cdzDeclines = pkgs.runCommand "cdz-declines"
          { nativeBuildInputs = [ xtaskCodegenDeclinesBin seedCompiler rustToolchain ]; } ''
          set -euo pipefail
          mkdir -p "$out"
          export CDZ_REPO_ROOT="${declinesSexpSrc}"
          export CDZ_SEED_BIN_DIR="${seedCompiler}/bin"
          xtask-codegen-declines "$out/declines_generated.rs"
        '';
        # DRIFT-GUARD: build-time-generated declines_generated.rs MUST be byte-identical to the committed one
        # (diff-style, mirrors cdzWasmAbiMatch). A forgotten regen after a data/unsupported.sexp edit → LOUD red.
        cdzDeclinesMatch = pkgs.runCommand "cdz-declines-match" { } ''
          set -euo pipefail
          if diff ${cdzDeclines}/declines_generated.rs ${./implementation/seed/crates/rcdzc/src/diag/declines_generated.rs} > declines.diff; then
            echo "ok: cdzDeclines (build-time codegen) == committed rcdzc/src/diag/declines_generated.rs (byte-identical)" > "$out"
          else
            echo "DRIFT: build-time declines codegen != committed declines_generated.rs — regen committed or fix data/unsupported.sexp:"
            cat declines.diff; exit 1
          fi
        '';
        # ORACLE-CHECK (operator's INVERTED guarantee, #5316): assert every opcode/valtype/section/magic byte in
        # the authored wasm-abi.sexp matches the wasm-encoder spec oracle — a derived test that catches a sexpr
        # transcription typo (the sexp is now the source of truth, so it must be cross-checked against the crate
        # that defines the real bytes). Needs cdz (decode the sexp) + the bin (carries the wasm-encoder oracle).
        wasmAbiOracle = pkgs.runCommand "cdz-wasm-abi-oracle"
          { nativeBuildInputs = [ xtaskCodegenWasmAbiBin seedCompiler ]; } ''
          set -euo pipefail
          export CDZ_REPO_ROOT="${wasmAbiSexpSrc}"
          export CDZ_SEED_BIN_DIR="${seedCompiler}/bin"
          xtask-codegen-wasm-abi --oracle-check | tee "$out"
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
            # NATIVE-COMPOUND (v-nix 2026-08-29): the three `cdz rewrite` patterns below match records in the
            # native-compound `#record((= k v) …)` form, NOT the retired list-tagged `("record" …)` form. The
            # M2 native-compound migration (v-ast-compound; reader-side #5223) changed how `cdz convert --from ml`
            # serializes records, so the old `("record" …)` patterns matched 0 nodes → the blob program→path +
            # deps-inject + contract-id rewrites SILENTLY no-op'd → every program-blob harness run failed at load
            # ("blob … must give exactly one of bytes or path"). #5223 fixed the harness READERS; this fixes the
            # WRITER-side rewrites here. NB field-pairs `(= k v)` are only matchable INSIDE a `#record(…)` (not
            # standalone), so the contract rewrite matches the CONTAINING record `#record((= contract …) ,@rest)`.
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
                ${seedCompiler}/bin/cdz rewrite '#record((= name ,nm) (= program "${n}"))' '#record((= name ,nm) (= path "${harnessPrograms.${n}}"))' \
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
                # POSITION-INDEPENDENT (v-nix 2026-08-29): match the contract field ANYWHERE in its record via
                # ,@pre + ,@post — a `#record((= contract …) ,@rest)` (contract-FIRST) pattern MISSED specs where
                # contract is not the first field (e.g. reducer-graph-cdz-forward `{ from, contract, to }`,
                # pure-run-emit-then-close), leaving the NAME unresolved → the itest rejected "field contract has
                # the wrong shape (expected a base62 contract-id)". ,@pre/,@post capture the surrounding fields so
                # the rewrite fires regardless of position + preserves them.
                ${seedCompiler}/bin/cdz rewrite "#record(,@pre (= contract \"${cname}\") ,@post)" "#record(,@pre (= contract \"$id\") ,@post)" \
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
                deps="$deps #record((= path \"$f\"))"
              done
              ${seedCompiler}/bin/cdz rewrite '#record((= registry ,reg) ,@rest)' \
                "#record((= registry ,reg) (= deps #list($deps)) ,@rest)" \
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
        # The rc-trace runtime variant's content address — for STORE placement + `--runtime` resolution only
        # (cdz-run --rc-trace loads it by path); NOT pinned by any gate, so it is deliberately NOT a
        # runtime_hash.rs const (that would force an xtask codegen emit; rc-trace stays flake-only, zero-xtask).
        runtimeRctraceHash = hashOf runtimeRctrace "cdz-runtime-rctrace-hash";
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
            # `--emit-diagnostics` writes the KIND_DIAGNOSTICS wire (the well-formedness fault set, with any
            # fixes) to `$out/diagnostics` UNCONDITIONALLY (even on error/decline — it exits the normal compile
            # status), so the exec can grade a case's `(fix …)`/`(count …)` diagnostic-QUALITY assertions (C1).
            if cdz-compile "''${inputs[@]}" "''${cfg[@]}" "''${entry[@]}" -t wasm -o "$out/emit.wasm" --emit-diagnostics "$out/diagnostics" 2>"$out/compile.err"; then
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
        # ── AOT corpus-exec (seq-250/271, #5893/5910/5922): precompile guest+runtime+store deps to .cwasm ONCE
        # (cranelift-ON, CA-cached), then EXEC cranelift-FREE via deserialize — removes the per-run wasmtime JIT
        # from the corpus exec (the recurring cranelift cost the operator flagged). Proven grade-identical to the
        # direct JIT path on heap cases (v-nix acceptance test). Peer (--peer) cases stay on the JIT path (not
        # precompiled-capable yet).
        # componentStoreCwasm — componentStore + each <hash>.wasm ALSO precompiled to <hash>.cwasm, so the
        # cranelift-free exec resolves store deps (NFC today) via <hash>.cwasm in --precompiled mode (#5922).
        componentStoreCwasm = pkgs.runCommand "cdz-component-store-cwasm"
          {
            nativeBuildInputs = [ cdzRun ];
            __contentAddressed = true;
            outputHashMode = "recursive";
            outputHashAlgo = "sha256";
          } ''
          set -euo pipefail
          cp -rL ${componentStore} "$out"; chmod -R u+w "$out"
          for w in "$out"/*.wasm; do
            cdz-run "$w" --precompile-out "''${w%.wasm}.cwasm"
          done
        '';
        # runtimeDebugCwasm — the debug-counters runtime precompiled ONCE, shared by every AOT exec (--runtime).
        runtimeDebugCwasm = pkgs.runCommand "cdz-runtime-debug-cwasm"
          { nativeBuildInputs = [ cdzRun ]; } ''
          cdz-run ${runtimeDebug} --precompile-out "$out"
        '';
        # mkCorpusPrecompile — per-case CA precompile of the guest emit.wasm → guest.cwasm (cranelift-ON, keyed
        # on the guest bytes → recompiles ONLY when the guest changes, not per exec rerun). SKIPS peer cases
        # (--peer not precompiled-capable) + no-emit (error/decline) cases; the exec JIT-falls-back for those.
        mkCorpusPrecompile = { name, build, idx }:
          pkgs.runCommand "corpus-precompile-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            # nullglob-SAFE peer detection (nix stdenv bash has nullglob ON — a bare `ls peer-*.wasm` with no
            # match lists CWD + exits 0, so `! ls …` wrongly reads as "no peers"; `find` is glob-independent).
            if [ -e ${build}/emit.wasm ] && [ -z "$(find ${build} -maxdepth 1 -name 'peer-*.wasm' -print -quit)" ]; then
              cdz-run ${build}/emit.wasm --precompile-out "$out/guest.cwasm"
            fi
          '';
        mkCorpusExec = { name, build, precompile, idx }:
          pkgs.runCommand "corpus-exec-${name}-${idx}"
            { } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            status=$(cat ${build}/compile.status)
            # Common grade args (outcome-independent). `--diagnostics` feeds KIND_DIAGNOSTICS so a case's
            # diagnostic-QUALITY facets are asserted (C1); `--component-name` names the emitted component.
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --baseline ${./spec/semantics/.gate-baseline})
            if [ -e ${build}/diagnostics ]; then args+=(--diagnostics ${build}/diagnostics); fi
            if [ -e ${build}/component-name ]; then args+=(--component-name "$(cat ${build}/component-name)"); fi
            if [ -e ${precompile}/guest.cwasm ]; then
              # AOT PATH (non-peer value case): CRANELIFT-FREE deserialize. Guest + debug runtime precompiled;
              # store deps (NFC) resolved as <hash>.cwasm from componentStoreCwasm (#5922). No per-run JIT — the
              # cranelift work happened ONCE in the (CA) precompile, cached per guest. Debug-counters runtime for
              # the heap-balance live-cell grade (scalar/const cases ignore the runtime, grade skips balance).
              export CDZ_STORE="${componentStoreCwasm}"
              ${cdzRunExec}/bin/cdz-run ${precompile}/guest.cwasm --precompiled --runtime ${runtimeDebugCwasm} "''${args[@]}"
            else
              # JIT FALLBACK: peer cases (--peer not precompiled-capable) + error/decline cases (no emit.wasm —
              # graded from the compile status, nothing to run). Byte-for-byte the pre-AOT behavior via cdzRun.
              export CDZ_STORE="${componentStore}"
              if [ -e ${build}/emit.wasm ]; then args=(${build}/emit.wasm "''${args[@]}"); fi
              for pw in ${build}/peer-*.wasm; do
                [ -e "$pw" ] || continue
                pn=$(basename "$pw" .wasm)               # peer-N
                args+=(--peer "$(cat ${build}/$pn.iface)=$pw")
              done
              args+=(--runtime ${runtimeDebug})
              ${cdzRun}/bin/cdz-run "''${args[@]}"
            fi
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
            # (v-corpus-harness #5766) The CADENZA-HOP path only: on a `(live-objects known-leak N)` case, a
            # count <= N PASSES (the cadenza round-trip reclaiming FEWER cells than the direct-wasm path is
            # strictly-SAFER — no leak, just tighter reclaim); > N still fails. This is CADENZA-ONLY: the direct
            # wasm `mkCorpusExec` deliberately stays EXACT `== N` as the leak drift-guard (a direct-path count
            # change — either direction — must red). Clears e.g. 13-strings 0023 (a benign hop reclaiming 3 fewer).
            args+=(--tolerate-fewer-live-objects)
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

        # ── quote-corpus: the QUOTE binary-AST round-trip pass (v-quote-corpus, design
        # DESIGN-quote-corpus-roundtrip-pass.md) ─────────────────────────────────────────────────────────
        # A SECOND exec layer over a DISTINCT shred: for each ELIGIBLE corpus case the shred emits a §2
        # two-export round-trip COMPONENT (`encodeQuoted() -> list<u8>` + `decodeCheck(list<u8>) -> bool`
        # around `quote E`) + its imposed `wit-world.ast` + `component-name`; the build compiles it; the exec
        # is the CALLER — `cdz-run --quote-roundtrip <iface>` threads `encode-quoted()`'s bytes back into
        # `decode-check(bytes)` (assert true) + a corrupt-bytes negative trial (assert false/trap), the
        # anti-const-fold caller-boundary round-trip. Mirrors mkCorpusCadenza's shred→build→exec caching graph.

        # SHRED (distinct from mkCorpusShred — a DIFFERENT program via `--quote-wrap`; cache-keyed on file +
        # cdzCorpus). Emits ELIGIBLE cases only (single-component; skips sibling-module/peer package cases),
        # keeping the base-corpus NNNN index — so the per-idx dir may be ABSENT (an ineligible case → the
        # build's `quote-skip` path handles it).
        mkQuoteCorpusShred = { name, file }:
          pkgs.runCommand "quote-corpus-shred-${name}"
            {
              nativeBuildInputs = [ cdzCorpus ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            cp ${file} ${name}.sexp
            cdz-corpus records --out-dir "$out" --quote-wrap ${name}.sexp
          '';

        # BUILD (content-addressed) — compile ONE eligible case's §2 round-trip program (+ its imposed
        # wit-world + component-name) to wasm, capturing the outcome. An ABSENT idx dir = an ineligible case
        # the shred skipped → a `quote-skip` marker (exec skips). A DECLINE (quote can't yet reify E — a
        # collection literal / a `def`/`export` compound) is NOT a derivation failure: `emit.wasm` is absent,
        # `compile.status` != 0 → the exec grades it Todo.
        mkQuoteCorpusBuild = { name, shred, idx }:
          pkgs.runCommand "quote-corpus-build-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            # nix stdenv bash has nullglob ON, so an unmatched glob expands to EMPTY (not the literal) → an
            # absent idx (ineligible, shred-skipped) yields no `case` dir. Mark quote-skip so the exec skips.
            case=$(echo ${shred}/${name}/${idx}-*)
            if [ -z "$case" ] || [ ! -d "$case" ]; then
              touch "$out/quote-skip"
              echo "skip (ineligible / not shredded): case ${idx} of ${name}" > "$out/why"
              exit 0
            fi
            cfg=("wit-world:w=$case/wit-world.ast" --component-name "$(cat "$case/component-name")")
            # Compile the two-export round-trip program. A refusal (quote reify gap) is captured, not fatal.
            if cdz-compile "ast:main=$case/program.ast" "''${cfg[@]}" -t wasm -o "$out/emit.wasm" 2>"$out/compile.err"; then
              printf '0' > "$out/compile.status"
            else
              printf '%s' "$?" > "$out/compile.status"
            fi
            cp "$case/component-name" "$out/component-name"
            cp "$case/description" "$out/description"
          '';

        # EXEC — the CALLER-boundary round-trip. Compiler-free (closure = cdzRun + the debug runtime).
        #   quote-skip (ineligible)                 → SKIP (exit 0).
        #   no emit.wasm (compile declined/refused) → TODO (exit 0; the quote-reify gap the pass DRIVES —
        #                                             flips to a real run as v-metaprogramming broadens reify).
        #   emit.wasm present                        → `cdz-run --quote-roundtrip <iface>`: PASS (exit 0) or
        #                                             FAIL (exit 1 → the derivation REDS — a genuine
        #                                             quote/codec/decode or anti-const-fold regression).
        # (Baseline/regression grading — Todo→Pass tracking — is a follow-up increment; this slice reds only
        #  on a compiled program whose round-trip breaks.)
        mkQuoteCorpusExec = { name, build, idx }:
          pkgs.runCommand "quote-corpus-exec-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            if [ -e ${build}/quote-skip ]; then
              echo "skip: quote-corpus ${name} case ${idx} — ineligible (module/peer or not shredded)" > "$out"
              exit 0
            fi
            if [ ! -e ${build}/emit.wasm ]; then
              echo "todo: quote-corpus ${name} case ${idx} — quote-wrap program declined to compile (quote reify gap): $(head -1 ${build}/compile.err 2>/dev/null)" > "$out"
              exit 0
            fi
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            # The caller-boundary round-trip: encode-quoted() → decode-check(bytes)==true + a corrupt-bytes
            # negative trial (==false/trap). A failed trial exits 1 → this derivation reds.
            cdz-run ${build}/emit.wasm --quote-roundtrip "$(cat ${build}/component-name)" --runtime ${runtimeDebug}
            echo "ok: quote-corpus ${name} case ${idx} — round-trip PASS ($(cat ${build}/description))" > "$out"
          '';

        quoteCorpusCaseChecks = { name, file }:
          let
            shred = mkQuoteCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in
          builtins.listToAttrs (map
            (idx: {
              name = "${name}-${idx}";
              value = mkQuoteCorpusExec {
                inherit name idx;
                build = mkQuoteCorpusBuild { inherit name shred idx; };
              };
            })
            idxs);

        mkQuoteCorpusFileAgg = { name, file }:
          let cases = quoteCorpusCaseChecks { inherit name file; };
          in
          assert (builtins.length (builtins.attrNames cases)) > 0;
          pkgs.runCommand "quote-corpus-${name}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues cases)}
            echo "ok: quote-corpus ${name} — ${toString (builtins.length (builtins.attrNames cases))} cases via per-case shred(--quote-wrap)→build→roundtrip-exec" > "$out"
          '';

        quoteCorpusFileAggs = builtins.listToAttrs (map
          (f:
            let stem = pkgs.lib.removeSuffix ".sexp" f; in
            {
              name = "quote-corpus-${stem}";
              value = mkQuoteCorpusFileAgg { name = stem; file = ./spec/semantics + "/${f}"; };
            })
          corpusFileNames);
        quoteCorpusAll = pkgs.runCommand "quote-corpus-all" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues quoteCorpusFileAggs)}
          echo "ok: quote-corpus — ${toString (builtins.length corpusFileNames)} files, per-case quote binary-AST round-trip (design-quote-corpus-roundtrip-pass)" > "$out"
        '';

        # ── quote-corpus VERDICT harvest (inc-4, mirrors mkCorpusVerdict/`.#corpus-verdicts`; v-corpus-harness
        # reviews for parity) — the `.quote-gate-baseline` regenerator input. Per eligible case, CLASSIFY the
        # verdict (compile-declined → `todo`; else the round-trip's `cdz-run --quote-roundtrip --emit-verdict`
        # tag `pass`/`fail`), write `<tag>\t<description>`, ALWAYS exit 0 (classify, not compare). An INELIGIBLE
        # (quote-skip) case emits NO line (it is not a quote-corpus case → absent from the baseline; the header
        # documents the single-component eligibility so an absence is not read as drift).
        mkQuoteCorpusVerdict = { name, build, idx }:
          pkgs.runCommand "quote-corpus-verdict-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            : > "$out"
            if [ -e ${build}/quote-skip ]; then
              exit 0   # ineligible (module/peer/not-shredded) → no verdict line
            fi
            desc=$(cat ${build}/description)
            if [ ! -e ${build}/emit.wasm ]; then
              # compile DECLINED (quote-reify gap) → todo (the pass DRIVES this; flips to pass as reify broadens)
              printf 'todo\t%s\n' "$desc" > "$out"
              exit 0
            fi
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            # CLASSIFY the round-trip: `pass` (both trials) / `fail` (a trial broke) — cdz-run writes the tag,
            # ALWAYS exit 0 (a fail emits its verdict, does not fail the derivation — the whole-corpus check
            # compares against the baseline). A rare `fail` is a real quote/codec/decode regression.
            cdz-run ${build}/emit.wasm --quote-roundtrip "$(cat ${build}/component-name)" \
              --emit-verdict "$TMPDIR/tag" --runtime ${runtimeDebug}
            printf '%s\t%s\n' "$(cat "$TMPDIR/tag")" "$desc" > "$out"
          '';

        quoteCorpusVerdictsFileAgg = { name, file }:
          let
            shred = mkQuoteCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
            cases = map
              (idx: mkQuoteCorpusVerdict { inherit name idx; build = mkQuoteCorpusBuild { inherit name shred idx; }; })
              idxs;
          in
          pkgs.runCommand "quote-corpus-verdicts-${name}" { } ''
            : > "$out"
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} >> "$out"'') cases}
          '';

        # `.#quote-corpus-verdicts` — the WHOLE-pass harvest (`<tag>\t<description>`, tag ∈ pass/todo; a `fail`
        # is a regression signal). The input a `save`/regenerator writes `.quote-gate-baseline` from. Eligibility:
        # single-component cases only (module/peer skipped) — record that in the baseline header so a "missing"
        # case is not mistaken for drift. 🚨 A `--save` MUST reject a `fail`-SPIKE (nix build-phase starvation
        # contamination, v-corpus-harness #6835) — check `grep -c '^fail' new` vs committed before baking.
        quoteCorpusVerdictsAll = pkgs.runCommand "quote-corpus-verdicts" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${quoteCorpusVerdictsFileAgg { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
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
              value =
                let build = mkCorpusBuild { inherit name shred idx; };
                in mkCorpusExec {
                  inherit name idx build;
                  precompile = mkCorpusPrecompile { inherit name build idx; };
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
        # ── corpus emit.wasm WARM TARGET (v-wasm-oracle, for v-nix's cache-warm.yml) ────────────────────────
        # Realize every per-case `emit.wasm` across the whole corpus so a CI cache-warm run pushes the full
        # per-case emit set to cachix. mkCorpusBuild is `__contentAddressed`, so this dedups with the corpus
        # gate/harvest and, once warm, makes the wasm-oracle emit-extraction harness (oracleWasmCaseDirs, which
        # reuses these exact mkCorpusBuild outputs) near-free — unblocking the uncapped full-corpus Core↔wasm
        # differential. `cat`-ing each build's marker into `$out` adds the store dependency (string context) so
        # building this attr forces the whole per-case emit graph WITHOUT a buildInput.
        # The per-case emit.wasm build store paths for the WHOLE corpus (~thousands of cases). This is
        # deliberately materialized as a `writeText` MANIFEST (a single store path) rather than an env attr:
        # passing the list through the process environment stringifies it into one huge `builds=…` env var,
        # which overflows the execve arg+env limit at full-corpus scale (`Argument list too long`, E2BIG —
        # the aggregator failed exactly this way on the first full run). The manifest's string context still
        # references every build, so realizing this derivation realizes (and thus cache-warms) them all.
        corpusEmitWasmBuilds = pkgs.lib.concatLists (map
          (f:
            let
              stem = pkgs.lib.removeSuffix ".sexp" f;
              file = ./spec/semantics + "/${f}";
              shred = mkCorpusShred { name = stem; inherit file; };
              n = corpusCaseCount file;
            in
            builtins.genList
              (i: mkCorpusBuild { name = stem; inherit shred; idx = pkgs.lib.fixedWidthNumber 4 i; })
              n)
          corpusFileNames);
        corpusEmitWasmWarm = pkgs.runCommand "corpus-emit-wasm-warm" { } ''
          cp ${pkgs.writeText "corpus-emit-wasm-builds"
            (pkgs.lib.concatStringsSep "\n" corpusEmitWasmBuilds)} "$out"
          echo "corpus-emit-wasm-warm: realized $(wc -l < "$out") per-case corpus emit.wasm builds" >&2
        '';
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

        # ── SYNTAX corpus (spec/syntax/) per-case gate (DESIGN-parser-test-corpus.md §4.1, inc-3c).
        # Unlike the semantics corpus, spec/syntax is DIRECTORY-per-case, so each case dir is ALREADY the
        # cache-isolation unit — NO shred/count/idx dance (that is bespoke to a semantics .sexp packing many
        # cases). ONE derivation per case dir, inputs = {that case dir, the `cdz` front-end} ONLY — NEVER the
        # parent spec/syntax tree, or an edit to any case would rotate EVERY case (the invariant that makes
        # the cache work). Cases enumerated at EVAL time via `readDir` (no IFD), like `corpusFileNames`.
        syntaxSurfaces = builtins.filter
          (n: (builtins.readDir ./spec/syntax).${n} == "directory")
          (builtins.attrNames (builtins.readDir ./spec/syntax));
        syntaxCases = builtins.concatMap
          (surface:
            let dir = ./spec/syntax + "/${surface}"; in
            map (caseName: { inherit surface caseName; })
              (builtins.filter (n: (builtins.readDir dir).${n} == "directory")
                (builtins.attrNames (builtins.readDir dir))))
          syntaxSurfaces;
        # Per-case CLASSIFY derivation — emits ONLY the verdict line `<verdict>\t<title>`, NO baseline
        # (kept out of the per-case inputs so a baseline edit re-runs just the cheap aggregate fold, not
        # every case). Mirrors `gate_syntax::grade_case`: `cdz convert --to sexpr --structural` vs
        # `tree.sexp`, `cdz fmt --stdout` vs `format.<ext>`-or-`input`; a non-zero convert is a decline →
        # `todo`. ALWAYS exits 0 (classify, don't fail — the aggregate folds vs the baseline).
        mkSyntaxCase = { surface, caseName }:
          let
            caseDir = ./spec/syntax + "/${surface}/${caseName}";
            title = "${surface}/${caseName}";
          in
          pkgs.runCommand "syntax-case-${surface}-${caseName}"
            { nativeBuildInputs = [ seedCompiler ]; } ''
            set -uo pipefail
            case=${caseDir}
            input=$(echo "$case"/input.*)
            ext=''${input##*.}
            # Harden the single-input assumption (v-corpus-harness review): if a case dir ever lacks
            # `input.*`, the glob stays literal → `cdz` would fail → mis-classified as a decline (todo).
            # A missing input is a mis-authored case → fail, not a silent todo.
            if [ ! -e "$input" ]; then
              printf 'fail\t%s\n' "${title}" > "$out"
              exit 0
            fi
            if cdz convert --to sexpr --structural "$input" > tree.actual 2>/dev/null; then
              if [ ! -f "$case/tree.sexp" ]; then
                verdict=fail            # parses but no golden tree (a mis-authored decline) → fail
              elif cmp -s tree.actual "$case/tree.sexp"; then
                if [ -f "$case/format.$ext" ]; then expected="$case/format.$ext"; else expected="$input"; fi
                cdz fmt --stdout "$input" > fmt.actual 2>/dev/null
                if cmp -s fmt.actual "$expected"; then
                  verdict=pass
                  # CODEMOD goldens: each `normalize.<pass>.<ext>` must equal `cdz normalize --<pass>
                  # --stdout <input>` (same-surface). Mirrors gate_syntax::grade_case; extends free as
                  # more `cdz normalize` passes land. nullglob-safe (no match → loop body never runs).
                  for g in "$case"/normalize.*."$ext"; do
                    [ -e "$g" ] || continue
                    b=$(basename "$g"); b=''${b#normalize.}; pass=''${b%.$ext}
                    [ -n "$pass" ] || continue
                    if cdz normalize "--$pass" --stdout "$input" > norm.actual 2>/dev/null \
                       && cmp -s norm.actual "$g"; then :; else verdict=fail; fi
                  done
                else
                  verdict=fail
                fi
              else
                verdict=fail            # wrong tree
              fi
            else
              verdict=todo              # the reader DECLINES (a malformed / not-yet-realized surface)
            fi
            printf '%s\t%s\n' "$verdict" "${title}" > "$out"
          '';
        syntaxCaseDrvs = map mkSyntaxCase syntaxCases;
        # The aggregate: force every per-case verdict, concat into a harvest file, and FOLD it vs the
        # committed baseline through `gate-syntax --compare` — the SAME `check_baseline` the CLI `--check`
        # uses (single-sourced; the nix path never gets a divergent/weaker fold). `--baseline` is explicit
        # because `xtaskBin` runs outside a repo tree. A regression / vanished / failing verdict reds here.
        syntaxCorpus = pkgs.runCommand "syntax-corpus"
          { nativeBuildInputs = [ xtaskBin ]; } ''
          set -euo pipefail
          : > verdicts.txt
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} >> verdicts.txt'') syntaxCaseDrvs}
          xtask gate-syntax --compare verdicts.txt --baseline ${./spec/syntax/.gate-baseline}
          echo "ok: syntax-corpus — ${toString (builtins.length syntaxCases)} cases via per-case classify + baseline fold" > "$out"
        '';

        # ── --save HARVEST (v-xtask-decompose seq-202 gate-delete: the nix replacement for `cargo xtask gate
        # --save`). The gate `--save` regenerated `.gate-baseline` from the current corpus verdicts; instead of
        # a heavy in-process re-run, HARVEST the verdicts from the per-case nix graph (cached) + let a thin
        # `xtask-save-baseline` leaf (v-xtask, WIP) write the baseline. `cdz-run --emit-verdict PATH` (#5746)
        # writes this case's CURRENT verdict `<tag>\t<description>` (tag ∈ pass/todo/fail — the coarse vocab
        # `.gate-baseline` records) + ALWAYS exits 0 (it CLASSIFIES, not compares — so a regressed case emits its
        # new verdict instead of failing the derivation, which the grade path would). `mkCorpusVerdict` mirrors
        # `mkCorpusExec` (same build/store/runtime/peer inputs) but adds `--emit-verdict "$out"` → $out IS the
        # one-line verdict; the aggregates concat them. `.#corpus-verdicts` = the whole-corpus harvest file
        # xtask-save-baseline consumes (verdicts-file → .gate-baseline). WASM baseline here; the rust /
        # rust-async baselines get the same treatment over the corpus-rust / corpus-rust-async exec variants
        # (follow-up once the wasm harvest + xtask-save-baseline registration land).
        mkCorpusVerdict = { name, build, idx }:
          pkgs.runCommand "corpus-verdict-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            status=$(cat ${build}/compile.status)
            # SAME inputs as mkCorpusExec (the classify runs the same grade), plus --emit-verdict.
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --baseline ${./spec/semantics/.gate-baseline})
            if [ -e ${build}/diagnostics ]; then args+=(--diagnostics ${build}/diagnostics); fi
            if [ -e ${build}/emit.wasm ]; then args=(${build}/emit.wasm "''${args[@]}"); fi
            if [ -e ${build}/component-name ]; then args+=(--component-name "$(cat ${build}/component-name)"); fi
            for pw in ${build}/peer-*.wasm; do
              [ -e "$pw" ] || continue
              pn=$(basename "$pw" .wasm)
              args+=(--peer "$(cat ${build}/$pn.iface)=$pw")
            done
            args+=(--runtime ${runtimeDebug})
            # CLASSIFY: write `<tag>\t<description>` to $out + exit 0 (takes precedence over --baseline; a
            # regressed/todo case emits its CURRENT verdict rather than failing the build).
            args+=(--emit-verdict "$out")
            cdz-run "''${args[@]}"
            # $out is written by cdz-run (--emit-verdict). Guard against an empty write (a real bug would leave
            # it absent → the aggregate `cat` fails loud, catching a broken emit-verdict rather than a silent gap).
            [ -s "$out" ] || { echo "corpus-verdict ${name} ${idx}: cdz-run --emit-verdict wrote no verdict" >&2; exit 1; }
          '';

        # Per-FILE verdict harvest: concat every case's one-line verdict into one `<tag>\t<description>` file.
        # (Order-independent — xtask-save-baseline parses into a description→verdict map + sorts on serialize.)
        verdictsFileAgg = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
            cases = map
              (idx: mkCorpusVerdict { inherit name idx; build = mkCorpusBuild { inherit name shred idx; }; })
              idxs;
          in
          assert (builtins.length cases) > 0;
          pkgs.runCommand "corpus-verdicts-${name}" { } ''
            : > "$out"
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} >> "$out"'') cases}
          '';

        # `.#corpus-verdicts` — the WHOLE-corpus harvest: every file's verdict lines concatenated. This is the
        # input `apps.save-baseline` feeds to v-xtask's `xtask-save-baseline` leaf (verdicts-file → .gate-baseline).
        # Cached through the same per-case shred→build→verdict graph as `corpus`, so re-harvesting an unchanged
        # corpus is a store cache hit.
        corpusVerdictsAll = pkgs.runCommand "corpus-verdicts" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${verdictsFileAgg { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
        '';

        # ── COARSE per-FILE verdict harvest (v-nix + v-corpus-harness, re-baseline coarsening 2026-09-01) ──
        # WHY: the per-CASE verdict graph above (mkCorpusBuild = one __contentAddressed output PER CASE, ~10.7k
        # ×3 backends, feeding mkCorpusVerdict) makes the WHOLE-corpus harvest a HUGE content-addressed graph.
        # Realising it fires a realisation (.doi) query per CA output PER substituter → a network-bound
        # "live-but-sleeping" storm that never completes (the corpus-verdicts wedge; substitute=false timed out
        # at 5.5h, camshaft-only still storms). The GATE keeps the per-case granularity (fast incremental PR
        # gating — a one-case edit re-runs ONLY that case's CA build/exec). Only the HARVEST coarsens: ONE
        # derivation per FILE compiles + grades + emit-verdicts EVERY case in the file INTERNALLY (looping the
        # shred's case dirs), so the harvest graph is ~35 file outputs, NOT tens of thousands of per-case CA
        # outputs → realisation queries drop ~300× → the storm cannot recur regardless of substitute settings.
        # VERDICT LOGIC IS BYTE-IDENTICAL to mkCorpusBuild+mkCorpusVerdict: same cdz-compile invocation, same
        # cdz-run --grade --emit-verdict, same store/runtime/baseline/peer handling, same `cat verdict >> out`
        # concat — only the derivation GRANULARITY changes (each case is independent, so a case graded solo vs
        # in a per-file batch yields the IDENTICAL verdict). v-corpus-harness owns the parity acceptance test
        # (coarse file output == concat of the per-case verdicts) + reconcile + LAND; corpusVerdictsCoarseParity
        # below is the per-file byte-parity spike this rests on.
        mkCorpusVerdictsFileCoarse = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            expected = corpusCaseCount file;
          in
          pkgs.runCommand "corpus-verdicts-coarse-${name}"
            {
              nativeBuildInputs = [ cdzCompile cdzRun ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            : > "$out"
            n=0
            for case in ${shred}/${name}/*/; do
              [ -d "$case" ] || continue
              case="''${case%/}"
              work="$TMPDIR/work"; rm -rf "$work"; mkdir -p "$work"
              # --- compile (mirrors mkCorpusBuild EXACTLY) ---
              inputs=("ast:main=$case/program.ast")
              entry=()
              for m in "$case"/module-*.ast; do
                if [ -e "$m" ]; then
                  mn=$(basename "$m" .ast); mn=''${mn#module-}
                  inputs+=("ast:$mn=$m")
                  entry=(--entry main)
                fi
              done
              cfg=()
              if [ -e "$case/wit-world.ast" ]; then cfg+=("wit-world:w=$case/wit-world.ast"); fi
              if [ -e "$case/component-name" ]; then cfg+=(--component-name "$(cat "$case/component-name")"); fi
              if cdz-compile "''${inputs[@]}" "''${cfg[@]}" "''${entry[@]}" -t wasm -o "$work/emit.wasm" --emit-diagnostics "$work/diagnostics" 2>"$work/compile.err"; then
                status=0
              else
                status=$?
              fi
              for p in "$case"/peer-*.ast; do
                [ -e "$p" ] || continue
                pn=$(basename "$p" .ast)
                cdz-compile "ast:main=$p" --component-name "$(cat "$case/$pn.iface")" -t wasm \
                  -o "$work/$pn.wasm" 2>>"$work/compile.err" || true
              done
              # --- grade + emit-verdict (mirrors mkCorpusVerdict EXACTLY) ---
              args=(--grade "$case/test-run.ast" --compile-status "$status" --compile-diag "$work/compile.err"
                    --baseline ${./spec/semantics/.gate-baseline})
              if [ -e "$work/diagnostics" ]; then args+=(--diagnostics "$work/diagnostics"); fi
              if [ -e "$work/emit.wasm" ]; then args=("$work/emit.wasm" "''${args[@]}"); fi
              if [ -e "$case/component-name" ]; then args+=(--component-name "$(cat "$case/component-name")"); fi
              for pw in "$work"/peer-*.wasm; do
                [ -e "$pw" ] || continue
                pn=$(basename "$pw" .wasm)
                args+=(--peer "$(cat "$case/$pn.iface")=$pw")
              done
              args+=(--runtime ${runtimeDebug})
              args+=(--emit-verdict "$work/verdict")
              cdz-run "''${args[@]}"
              [ -s "$work/verdict" ] || { echo "corpus-verdicts-coarse ${name}: $case wrote no verdict" >&2; exit 1; }
              cat "$work/verdict" >> "$out"
              n=$((n + 1))
            done
            # Enumeration guard: the coarse loop MUST see exactly the eval-time case count (catches a shred vs
            # corpusCaseCount drift that would silently drop cases from the harvest).
            if [ "$n" -ne ${toString expected} ]; then
              echo "corpus-verdicts-coarse ${name}: graded $n cases, expected ${toString expected}" >&2; exit 1
            fi
          '';

        # `.#packages.corpus-verdicts-coarse` — the whole-corpus COARSE harvest (~35 file derivations), the
        # storm-free replacement for corpusVerdictsAll that apps.save-baseline will consume once v-corpus-harness
        # signs off the parity acceptance test. Kept SEPARATE from corpusVerdictsAll for now so the existing gate
        # + per-case harvest are untouched during the spike.
        corpusVerdictsCoarseAll = pkgs.runCommand "corpus-verdicts-coarse" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${mkCorpusVerdictsFileCoarse { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
        '';

        # PARITY SPIKE — the coarse per-file harvest MUST be byte-identical to the per-case verdictsFileAgg for
        # the SAME file (the v-corpus-harness acceptance test, one file). Sorts both before diffing because the
        # coarse loop walks the shred dirs in glob (numeric) order while verdictsFileAgg walks eval-time idxs —
        # both orderings are valid (xtask-save-baseline parses into a description→verdict map + re-sorts), so the
        # invariant is SET-equality of verdict lines, which sorted-diff checks. Green here = the coarsening
        # preserves every verdict; then v-corpus-harness widens to the whole corpus + the 3 backends + LANDs.
        # Parametrized per-file (v-corpus-harness acceptance: run a DIVERSE sample, not the storm-prone whole
        # corpus at once — the per-case reference verdictsFileAgg is itself the storm-prone graph at 35-file
        # scale). `.#corpus-verdicts-coarse-parity-<stem>` runs it for any file.
        mkCoarseParity = { name, file }:
          pkgs.runCommand "corpus-verdicts-coarse-parity-${name}" { } ''
            if diff <(sort ${mkCorpusVerdictsFileCoarse { inherit name file; }}) \
                    <(sort ${verdictsFileAgg { inherit name file; }}); then
              echo "ok: coarse per-file harvest byte-identical to per-case for ${name}" > "$out"
            else
              echo "PARITY FAIL: coarse != per-case verdicts for ${name}" >&2; exit 1
            fi
          '';
        corpusVerdictsCoarseParity = mkCoarseParity {
          name = pkgs.lib.removeSuffix ".sexp" (builtins.head corpusFileNames);
          file = ./spec/semantics + "/${builtins.head corpusFileNames}";
        };

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

        # ── RUST verdict harvest (v-xtask-decompose, the flake.nix:3514 follow-up to the wasm harvest) —
        # mirrors mkCorpusVerdict/`.#corpus-verdicts` for the rust backend. Per case, CLASSIFY the verdict via
        # `cdz-rust-run --emit-verdict` (#6978): it writes `<tag>\t<description>` (tag ∈ pass/todo/fail from the
        # shared cdz-corpus-grade `verdict()` — the same coarse vocab `.gate-baseline-rust` records) and ALWAYS
        # exits 0 — classify, not compare, so a regressed/todo case emits its CURRENT verdict rather than failing
        # the derivation (drops the `--baseline` mkCorpusRustExec passes). Same build inputs as mkCorpusRustExec.
        mkCorpusRustVerdict = { name, build, idx }:
          pkgs.runCommand "corpus-rust-verdict-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRustRun rustToolchain ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            mkdir -p "$TMPDIR/w"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --compile-status "$status" --compile-diag ${build}/compile.err
                  --cdz-rt-dir ${rustRlibs} --cdz-num-dir ${rustRlibs} --cadenza-ast-dir ${rustRlibs}
                  --workdir "$TMPDIR/w" --emit-verdict "$out")
            if [ -e ${build}/emit.rs ]; then args+=(--module ${build}/emit.rs); fi
            cdz-rust-run "''${args[@]}"
            # $out is written by cdz-rust-run (--emit-verdict). Guard an empty write (a real bug would leave it
            # absent → the aggregate `cat` fails loud, catching a broken emit-verdict rather than a silent gap).
            [ -s "$out" ] || { echo "corpus-rust-verdict ${name} ${idx}: cdz-rust-run --emit-verdict wrote no verdict" >&2; exit 1; }
          '';

        verdictsRustFileAgg = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
            cases = map
              (idx: mkCorpusRustVerdict { inherit name idx; build = mkCorpusRustBuild { inherit name shred idx; }; })
              idxs;
          in
          assert (builtins.length cases) > 0;
          pkgs.runCommand "corpus-verdicts-rust-${name}" { } ''
            : > "$out"
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} >> "$out"'') cases}
          '';

        # `.#corpus-verdicts-rust` — the WHOLE-corpus RUST verdict harvest (one `<tag>\t<description>` line per
        # case). `apps.save-baseline` feeds this to xtask-save-baseline to regenerate `.gate-baseline-rust`.
        corpusRustVerdictsAll = pkgs.runCommand "corpus-verdicts-rust" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${verdictsRustFileAgg { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
        '';

        # ── RUST-ASYNC verdict harvest (twin of the rust one; the exec adds `--async` + reads the async
        # signature marker, exactly like mkCorpusRustAsyncExec). Feeds `.gate-baseline-rust-async`.
        mkCorpusRustAsyncVerdict = { name, build, idx }:
          pkgs.runCommand "corpus-rust-async-verdict-${name}-${idx}"
            {
              nativeBuildInputs = [ cdzRustRun rustToolchain ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            mkdir -p "$TMPDIR/w"
            status=$(cat ${build}/compile.status)
            args=(--grade ${build}/test-run.ast --async --compile-status "$status" --compile-diag ${build}/compile.err
                  --cdz-rt-dir ${rustRlibs} --cdz-num-dir ${rustRlibs} --cadenza-ast-dir ${rustRlibs}
                  --workdir "$TMPDIR/w" --emit-verdict "$out")
            if [ -e ${build}/emit.rs ]; then args+=(--module ${build}/emit.rs); fi
            cdz-rust-run "''${args[@]}"
            [ -s "$out" ] || { echo "corpus-rust-async-verdict ${name} ${idx}: cdz-rust-run --emit-verdict wrote no verdict" >&2; exit 1; }
          '';

        verdictsRustAsyncFileAgg = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
            cases = map
              (idx: mkCorpusRustAsyncVerdict { inherit name idx; build = mkCorpusRustAsyncBuild { inherit name shred idx; }; })
              idxs;
          in
          assert (builtins.length cases) > 0;
          pkgs.runCommand "corpus-verdicts-rust-async-${name}" { } ''
            : > "$out"
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} >> "$out"'') cases}
          '';

        # `.#corpus-verdicts-rust-async` — the WHOLE-corpus RUST-ASYNC verdict harvest. Feeds
        # `.gate-baseline-rust-async` via `apps.save-baseline`.
        corpusRustAsyncVerdictsAll = pkgs.runCommand "corpus-verdicts-rust-async" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${verdictsRustAsyncFileAgg { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
        '';

        # ── COARSE per-FILE RUST + RUST-ASYNC verdict harvest (v-nix, coarsening 2026-09-01) — the rust twin of
        # mkCorpusVerdictsFileCoarse. Same storm rationale: the per-case rust graph (mkCorpusRustBuild = one
        # __contentAddressed output per case → mkCorpusRustVerdict) is a huge CA graph. This coarsens the HARVEST
        # to ONE derivation per file (loops the shred case dirs, compiles -t rust + grades via cdz-rust-run
        # --emit-verdict internally). Verdict logic is byte-identical to mkCorpusRustBuild+mkCorpusRustVerdict
        # (same cdz-compile -t rust, same cdz-rust-run --grade --emit-verdict + rustRlibs dirs; `async` adds
        # --async exactly like mkCorpusRustAsyncVerdict). The GATE (corpus-rust / corpus-rust-async per-case
        # checks) is untouched. rust has NO peers/wit-world/component-name/diagnostics, so the loop body is the
        # simpler rust shape.
        mkCorpusRustVerdictsFileCoarse = { name, file, async ? false }:
          let
            shred = mkCorpusShred { inherit name file; };
            expected = corpusCaseCount file;
            tag = if async then "rust-async" else "rust";
          in
          pkgs.runCommand "corpus-verdicts-${tag}-coarse-${name}"
            {
              nativeBuildInputs = [ cdzCompile cdzRustRun rustToolchain ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            : > "$out"
            n=0
            for case in ${shred}/${name}/*/; do
              [ -d "$case" ] || continue
              case="''${case%/}"
              work="$TMPDIR/work"; rm -rf "$work"; mkdir -p "$work/w"
              # --- compile (mirrors mkCorpusRust${if async then "Async" else ""}Build: -t ${if async then "rust-async" else "rust"}) ---
              inputs=("ast:main=$case/program.ast")
              entry=()
              for m in "$case"/module-*.ast; do
                if [ -e "$m" ]; then
                  mn=$(basename "$m" .ast); mn=''${mn#module-}
                  inputs+=("ast:$mn=$m")
                  entry=(--entry main)
                fi
              done
              if cdz-compile "''${inputs[@]}" "''${entry[@]}" -t ${if async then "rust-async" else "rust"} -o "$work/emit.rs" 2>"$work/compile.err"; then
                status=0
              else
                status=$?
              fi
              # --- grade + emit-verdict (mirrors mkCorpusRust${if async then "Async" else ""}Verdict) ---
              args=(--grade "$case/test-run.ast")
              ${pkgs.lib.optionalString async ''args+=(--async)''}
              args+=(--compile-status "$status" --compile-diag "$work/compile.err"
                     --cdz-rt-dir ${rustRlibs} --cdz-num-dir ${rustRlibs} --cadenza-ast-dir ${rustRlibs}
                     --workdir "$work/w" --emit-verdict "$work/verdict")
              if [ -e "$work/emit.rs" ]; then args+=(--module "$work/emit.rs"); fi
              cdz-rust-run "''${args[@]}"
              [ -s "$work/verdict" ] || { echo "corpus-verdicts-${tag}-coarse ${name}: $case wrote no verdict" >&2; exit 1; }
              cat "$work/verdict" >> "$out"
              n=$((n + 1))
            done
            if [ "$n" -ne ${toString expected} ]; then
              echo "corpus-verdicts-${tag}-coarse ${name}: graded $n cases, expected ${toString expected}" >&2; exit 1
            fi
          '';

        corpusRustVerdictsCoarseAll = pkgs.runCommand "corpus-verdicts-rust-coarse" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${mkCorpusRustVerdictsFileCoarse { name = stem; file = ./spec/semantics + "/${f}"; }} >> "$out"'')
              corpusFileNames}
        '';
        corpusRustAsyncVerdictsCoarseAll = pkgs.runCommand "corpus-verdicts-rust-async-coarse" { } ''
          : > "$out"
          ${pkgs.lib.concatMapStringsSep "\n"
              (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
                ''cat ${mkCorpusRustVerdictsFileCoarse { name = stem; file = ./spec/semantics + "/${f}"; async = true; }} >> "$out"'')
              corpusFileNames}
        '';

        # PARITY SPIKES (rust + rust-async): coarse per-file == per-case verdictsRust{,Async}FileAgg, sorted-diff.
        # Parametrized per-file (v-corpus-harness: 01-literals scalar-only is NOT enough — the -t rust-async
        # variant bug proved diverse shapes must be exercised). `.#corpus-verdicts-rust{,-async}-coarse-parity-<stem>`.
        mkRustCoarseParity = { name, file, async ? false }:
          let tag = if async then "rust-async" else "rust";
          in pkgs.runCommand "corpus-verdicts-${tag}-coarse-parity-${name}" { } ''
            if diff <(sort ${mkCorpusRustVerdictsFileCoarse { inherit name file async; }}) \
                    <(sort ${if async then verdictsRustAsyncFileAgg { inherit name file; } else verdictsRustFileAgg { inherit name file; }}); then
              echo "ok: coarse ${tag} per-file == per-case for ${name}" > "$out"
            else
              echo "PARITY FAIL: coarse ${tag} != per-case for ${name}" >&2; exit 1
            fi
          '';
        corpusRustVerdictsCoarseParity = mkRustCoarseParity {
          name = pkgs.lib.removeSuffix ".sexp" (builtins.head corpusFileNames);
          file = ./spec/semantics + "/${builtins.head corpusFileNames}";
        };
        corpusRustAsyncVerdictsCoarseParity = mkRustCoarseParity {
          name = pkgs.lib.removeSuffix ".sexp" (builtins.head corpusFileNames);
          file = ./spec/semantics + "/${builtins.head corpusFileNames}";
          async = true;
        };

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

        # corpus-NATIVIZE (v-fleet-tooling 2026-08-30, wiring v-corpus-harness's lint per concierge): the M3
        # INPUT-FORM guard — every corpus INPUT compound-value literal must be native #ctor form, not a classic
        # name-head like `(list …)`/`(map …)`. A non-native input reds a full `gate --check` fleet-wide only
        # after the ~15-30min gate; this catches it in seconds AND (folded into localGate below) HOLDs a
        # self-merge on a violation — the teeth a GitHub required-status can't give, since `gh pr merge
        # --admin` bypasses required checks (#6025 self-merged classic-form past the ADVISORY checks.yml job).
        # The one legit exemption (corpus-05 #6047, the #6042 name-head parity guard) is honored via the
        # single-source `nativize_compound_impl` marker cdz-corpus's `nativize-check` shares — green-confirmed
        # 34/34 by v-corpus-harness before this fold. Mirrors corpusVanishedCheck (own cdzCorpus, same file set).
        corpusNativizeCheck = pkgs.runCommand "corpus-nativize-check"
          { nativeBuildInputs = [ cdzCorpus ]; } ''
          set -euo pipefail
          cdz-corpus nativize-check ${
            pkgs.lib.concatMapStringsSep " " (f: "${./spec/semantics + "/${f}"}") corpusFileNames
          }
          echo "ok: corpus-nativize — all corpus files in native #ctor compound-value input form" > "$out"
        '';

        # capability-error (v-fleet-tooling gate-wiring 2026-08-31, scan v-corpus-harness #6924): the
        # impl-independent-spec guard — a corpus case must NOT pin a CAPABILITY-LIMIT code (CDZ0900,
        # should-work-but-unimplemented) as an `(error …)` expectation, since that bakes a current impl
        # limit into the spec. `cdz-corpus capability-error-check` exits non-zero + names each offending
        # case. FOLDED into the localGate fail-set below → a new CDZ0900-pin HOLDs a self-merge (teeth a GHA
        # required-status can't give under `gh pr merge --admin`). Pure static parse (no compile/run), cheap
        # — same shape + closure (cdzCorpus) as corpusNativizeCheck. Starts GREEN: v-corpus-harness confirmed
        # 0 hits on the 34-file corpus, so it folds in immediately (no residue / fix-then-fold wait).
        capabilityErrorCheck = pkgs.runCommand "capability-error-check"
          { nativeBuildInputs = [ cdzCorpus ]; } ''
          set -euo pipefail
          cdz-corpus capability-error-check ${
            pkgs.lib.concatMapStringsSep " " (f: "${./spec/semantics + "/${f}"}") corpusFileNames
          }
          echo "ok: capability-error — no corpus case pins a capability-limit code (CDZ0900) as an (error …)" > "$out"
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

        # optSweepCheck — the TIERED-OPT LEVEL-EQUIVALENCE invariant (v-core-opt's OptLevel/PassManager on the
        # shared Core column). `cargo xtask gate --opt-sweep` compiles+runs EVERY corpus case at O0/O1/O2/O3 and
        # asserts observably-identical behavior (same value/trap/decline) — a SAME-RUN cross-level diff, NOT a
        # baseline compare, so NO --check (the gate ignores --save/--check under --opt-sweep). Default target
        # (wasm — the primary pipeline the Core passes' first consumer targets). It exits NON-ZERO on any O0..O3
        # divergence (a candidate miscompile), so this derivation reds exactly when the invariant breaks. Mirrors
        # gateCheck (same warm release-deps + store); the only difference is --opt-sweep vs --check. ~4x a corpus
        # run (4 levels/case) — UNSHARDED because a nix check has no free-CI-runner reclaim window (the sole
        # reason the nightly RUST gate shards); it runs to completion + caches in-store/cachix + rebuilds only on
        # a corpus/compiler change. ADVISORY: exposed as `checks.<sys>.opt-sweep` but NOT in the localGate
        # fail-set — v-gha-green wires it into nightly.yml (advisory, within-a-day catch, proportionate to the
        # low-frequency Core-pass-change risk; concierge greenlit nightly-advisory 2026-08-31). Currently green
        # (v-core-opt: 1876 cases, 0 divergences). A future `--target rust` sibling is optional (v-core-opt).
        optSweepCheck = craneLib.mkCargoDerivation {
          pname = "cdz-opt-sweep";
          version = "0.0.0";
          src = gateSrc;
          cargoArtifacts = cargoArtifactsRelease;
          cargoVendorDir = seedCargoVendor;
          CARGO_PROFILE = "release";
          doInstallCargoArtifacts = false;
          nativeBuildInputs = [ pkgs.wasm-tools ];
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- gate --opt-sweep --store "${componentStore}"
          '';
          installPhaseCommand = ''
            echo "ok: cdz-opt-sweep (cargo xtask gate --opt-sweep --store <nix store> — O0..O3 level-equivalence, all corpus cases)" > "$out"
          '';
        };

        # gateCheckVerify — a SOLO full-corpus verify path with a GENEROUS per-case timeout (v-nix, for v-effects'
        # UAF-critical #5090 SITE-B verification, concierge 2026-08-29). WHY: gateCheck (above) sets NO
        # CDZ_RUN_TIMEOUT_SECS, so `gate --check` uses run_timeout()'s 30s DEFAULT per-case deadline — a
        # multi-file effects-grade case that folds >30s UNDER FLEET LOAD false-traps ("did not finish within
        # 30s"), so a heavy effects grade can't be solo-verified. This clone raises CDZ_RUN_TIMEOUT_SECS to 1800
        # (30 min/case) so a legitimately-slow case runs to completion; identical full-corpus grade otherwise
        # (same gateSrc + baselines + store). NOT a localGate constituent: a 30-min per-case ceiling is a
        # VERIFY affordance, NOT a merge gate (a real infinite loop must still be caught fast by the 30s gate).
        # And a plain `nix build .#checks.<sys>.gate-check-verify` runs with NO fleet batch-prefilter wall cap
        # (that 15-min cap is pr-sync's, not a solo build) → the two caps v-effects hit are both lifted here.
        # Strictly a timeout-RELAXATION of the green gateCheck → cannot newly fail a case gateCheck passes.
        gateCheckVerify = craneLib.mkCargoDerivation {
          pname = "cdz-gate-check-verify";
          version = "0.0.0";
          src = gateSrc;
          cargoArtifacts = cargoArtifactsRelease;
          cargoVendorDir = seedCargoVendor;
          CARGO_PROFILE = "release";
          CDZ_RUN_TIMEOUT_SECS = "1800";
          doInstallCargoArtifacts = false;
          nativeBuildInputs = [ pkgs.wasm-tools ];
          buildPhaseCargoCommand = ''
            cargo run --locked --package xtask --profile release -- gate --check --store "${componentStore}"
          '';
          installPhaseCommand = ''
            echo "ok: cdz-gate-check-verify (full-corpus gate --check, CDZ_RUN_TIMEOUT_SECS=1800 — solo verify, not a merge gate)" > "$out"
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
        # runs the STANDALONE `xtask-bench` crate (v-xtask-decompose, carved out of the xtask monolith) diffing
        # cdz-runtime's hot_op_allocation_ceilings vs spec/bench/.alloc-baseline — same measurement, off the
        # monolith. xtask-bench resolves the repo root from cwd (the crane build dir = benchSrc) when
        # CDZ_REPO_ROOT is unset, exactly as `cargo xtask bench` did.
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
            cargo run --locked --package xtask-bench --profile release
          '';
          installPhaseCommand = ''
            echo "ok: cdz-bench-check (xtask-bench, crane release-deps-cached)" > "$out"
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
            (map (c: ./implementation/seed/crates + ("/" + c)) cdzWasmCrateDirs) ++ [
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
            (map (c: ./implementation/seed/crates + ("/" + c)) cdzWasmCrateDirs) ++ [ ./rust-toolchain.toml ]);
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
              # LOWER the bulk-memory ops instead of stripping the feature DECLARATION. rustc/LLVM-22 emits
              # memory.copy/fill (bulk-memory) + the target_features section declaring them. The old
              # `--strip-target-features` removed the DECLARATION while leaving the memory.copy/fill INSTRUCTIONS
              # in place → the guide's jco/wasm runtime then won't enable bulk-memory → memory.copy TRAPS
              # out-of-bounds (guide-examples 252-OOB; v-guide-infra proved via `wasm-opt --print-features`:
              # stripped bytes error "memory.copy operations require bulk memory operations"). wasm-pack works
              # because its older binaryen -Os AUTO-LOWERED these; binaryen 130/131 -Os stopped. FIX (v-guide-
              # infra 2026-09-01): run `--llvm-memory-copy-fill-lowering` WHILE the target_features section is
              # still present (i.e. on the -Os output, before/instead of any strip) — it rewrites memory.copy/
              # fill to bounds-safe loops AND drops the bulk-memory feature, so the final module has NEITHER the
              # bulk-memory instructions NOR the feature declaration → resolves BOTH the runtime OOB (the 252)
              # AND the jco section-rejection (the earlier 25). Supersedes the strip. (binaryen VERSION was a
              # red herring — 117-vs-131 didn't matter; the strip-vs-lower difference did.)
              wasm-opt --llvm-memory-copy-fill-lowering pkg/cdz_wasm_bg.wasm -o pkg/cdz_wasm_bg.wasm
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
            xtaskCodegenGuideBin # I5: the prebuilt guide sexp→TSX codegen bin on PATH (+ CDZ_XTASK_CODEGEN_GUIDE)
          ];
          # npmConfigHook reads these: the vendored dep set + the dir holding package-lock.json.
          npmDeps = guideNpmDeps;
          npmRoot = "guide";
          # The base the guide's vite build fingerprints assets under — mirror the GHA env
          # (VITE_BASE=/<repo>/). The repo is `cadenza`; a bundle-path check reads it.
          VITE_BASE = "/cadenza/";
          # I5 (v-guide-infra whole-guide→sexpr flip): the guide's `npm run codegen` (+ check:codegen[-sync])
          # regenerate the 42 @generated .tsx from the .sexp source-of-truth via the xtask-codegen-guide bin.
          # guideExamplesCheck has rustToolchain but NO cargo vendor, so an in-gate `cargo build -p` can't run
          # offline — instead point the guide's scripts at the PREBUILT bin (they resolve $CDZ_XTASK_CODEGEN_GUIDE
          # when set, else fall back to `cargo build -p` for local dev). Mirrors cdzWasmAbi/cdzPlatformContracts
          # consuming their carved codegen bins. xtaskCodegenGuideBin is also on PATH (nativeBuildInputs below).
          CDZ_XTASK_CODEGEN_GUIDE = "${xtaskCodegenGuideBin}/bin/xtask-codegen-guide";
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
              npm run check:prose-native
              # (v-guide-infra #5768) .sexp↔chapters.ts registry DRIFT gate: asserts every registry field
              # (title/blurb/pillar/section/exercises) is DERIVABLE from the sibling .sexp — the drift-gate for
              # the upcoming chapters.ts→@generated derivation. Pure node (reads guide/src + chapters.ts), no
              # cargo/xtask, zero new inputs (guideExamplesSrc already includes ./guide).
              npm run check:registry-derive
              # (v-guide-infra #5774) chapters.ts CHAPTERS[] is now @generated (codegen regenerates it from
              # chapter-order.mjs + the .sexp). check:registry-sync (codegen-registry.mjs --check) asserts the
              # COMMITTED chapters.ts byte-equals the generated output (like check:codegen-sync for the .tsx) —
              # the primary drift gate, which SUBSUMES check:registry-derive above (committed==generated ⇒ the
              # derived fields trivially match). Kept both (harmless; derive is the narrower field-level check).
              # Pure node, no cargo/xtask, zero new inputs.
              npm run check:registry-sync
              # (v-guide-infra fork1a) playground examples.ts EXAMPLES[] is now @generated from the sibling
              # examples.sexp (source-of-truth). Assert committed == regen — the playground analogue of
              # check:registry-sync above. Run via the Rust xtask DIRECTLY (operator: no JS shim); the
              # prebuilt xtaskCodegenGuideBin is on PATH (+ CDZ_XTASK_CODEGEN_GUIDE). cwd is guide/ (the
              # `cd guide` above), so the path is guide-relative. (v-nix pre-approved this line.)
              xtask-codegen-guide --playground-registry --check src/playground/examples.ts
              # seq-248 fork1b: assert the committed @generated HomePageExamples.ts == regen from HomePage.sexp
              # (direct xtask, no JS shim — same style as the playground-registry gate). Guards the .sexp→.ts drift.
              xtask-codegen-guide --homepage --check src/content/HomePage.sexp
              npm run check:diagnostics
              # (v-guide-infra) cdz-wasm QUERY-consumer guard: export_types/type_at/define_at/disposition must
              # DECODE their binary wires, not run a raw from_utf8 over them. cdz-wasm is outside the workspace +
              # ungated, so the binary-flip (#6148) silently broke the guide/editor consumers (fixed #6324/#6342);
              # this reds the gate on a future flip instead of in-browser. Runs in its OWN process (one program),
              # so it is independent of check:examples' accumulation. Pure node, no new inputs.
              npm run check:wasm-queries
              # (v-guide-infra) inline (cdz …) render guard: every AST-backed <Cadenza ast=…> span must
              # render in both surfaces from its embedded binary-AST (render_binary, #7245). check:examples/
              # guide-shred only gate runnable/exercise SOURCES, so a mis-rendering INLINE prose span was
              # ungated — this closes that gap. Pure node, reuses the staged pkg.
              npm run check:cdz-render
              npm run check:examples
              npm run check:calculator
              npm run check:worker-stack
              npm run check:tuple-collection
              npm run check:parameterized-entry
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

        # guideSite — the DEPLOYABLE static guide site (`guide/dist/`) built through NIX so the GitHub-Pages
        # deploy reuses the SHARED nix cache instead of a cold raw build every 30-min cron (operator directive
        # 2026-08-29, job 33275680429 — the pages deploy spent a lot of time building). Same build path as
        # guideExamplesCheck (consume the cached cdzWasmPkg + componentStore, stage-wasm, npm ci, npm run build
        # = codegen + tsc + vite), but WITHOUT the check:* battery (those gate in guideExamplesCheck / CI
        # separately — a pure site build) and OUTPUTS `dist/` instead of a verdict. Cache win: cdzWasmPkg +
        # componentStore + guideNpmDeps are all shared/cached, and this derivation is input-addressed on the
        # guide + compiler-wasm closure, so an unchanged-trunk cron deploy is a nix-store CACHE HIT (near-instant)
        # rather than a full ARM rebuild. The pages.yml workflow (v-gha-green) runs `nix build .#guide-site` and
        # uploads $out as the Pages artifact; v-guide-infra owns the guide build inputs + the codegen bin.
        guideSite = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-guide-site";
          version = "0.0.0";
          src = guideExamplesSrc;
          nativeBuildInputs = [
            rustToolchain
            pkgs.nodejs_22
            pkgs.npmHooks.npmConfigHook
            xtaskCodegenGuideBin
          ];
          npmDeps = guideNpmDeps;
          npmRoot = "guide";
          # The Pages base path (project page https://<owner>.github.io/cadenza/) — same as guideExamplesCheck.
          VITE_BASE = "/cadenza/";
          CDZ_XTASK_CODEGEN_GUIDE = "${xtaskCodegenGuideBin}/bin/xtask-codegen-guide";
          buildPhase = ''
            runHook preBuild
            # Consume the cached browser-compiler wasm pkg + stage it and the value-heap runtime into guide/
            # (identical to guideExamplesCheck steps 1-2).
            cp -r ${cdzWasmPkg} implementation/seed/crates/cdz-wasm/pkg
            chmod -R u+w implementation/seed/crates/cdz-wasm/pkg
            export CADENZA_STORE="${componentStore}"
            node guide/scripts/stage-wasm.mjs
            # Build the deployable site: `npm run build` = `npm run codegen` (regenerate the @generated .tsx +
            # chapters.ts via CDZ_XTASK_CODEGEN_GUIDE) + `tsc -b` + `vite build` → guide/dist/. No check:* battery
            # (gated in guideExamplesCheck/CI) — this is the pure Pages artifact build.
            ( cd guide
              npm ci
              patchShebangs node_modules
              npm run build
            )
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp -r guide/dist "$out"
            runHook postInstall
          '';
        };

        # ── guide-examples SHRED (operator directive 2026-08-28: the serial check:examples takes 10min+;
        # examples never change → SHRED it like the corpus, heavily cached + parallel). v-guide-infra owns the
        # shred CLI (scripts/shred-examples.mjs, #5091/#5096 — deterministic dir names, no timestamps); v-nix
        # owns the nix wiring. This FOUNDATION derivation runs the CLI ONCE into per-example artifact dirs
        # (mirrors mkCorpusShred). Content-addressed → the per-case build/exec layer (a follow-up, once the
        # eval-time case enumeration is settled with v-guide-infra) keys on these exact bytes + caches.
        #
        # The CLI is plain node (≥22.6 for .ts type-stripping) + the browser compiler wasm for `render_syntax`
        # ONLY (surface conversion sexpr↔ml — it does NOT compile or run, so NO runtime store / npm ci needed,
        # unlike guideExamplesCheck). It loads guide/src/wasm/pkg/cdz_wasm.js, so stage cdzWasmPkg (the #5089-
        # fixed browser-compiler pkg) there. `cargo xtask guide-wasm` is the non-nix equivalent of that staging.
        # seq-248 (fork1 complete): the guide shred is now the RUST `xtask-codegen-guide --shred` (from the
        # binary AST), NOT the node scripts/shred-examples.mjs. The caller converts each doc .sexp → binary AST
        # (`cdz convert --from sexpr --to binary`) and feeds them; the shred auto-detects each .cdzb's doc type
        # (chapter runnable/exercise · playground example · homepage runnable) and emits the per-case dirs +
        # manifest.json. FEED ORDER = case dir order: 42 chapters (alphabetical stem) → HomePage → 59 playground
        # examples (numeric-prefix order), reproducing the prior 0000-0350 chapters / 0351 homepage / 0352-0410
        # playground layout → 410 cases (coverage-neutral with the retired node shred). No node / no browser wasm.
        guideShred = pkgs.stdenvNoCC.mkDerivation {
          pname = "guide-shred";
          version = "0.0.0";
          src = pkgs.lib.fileset.toSource { root = ./guide; fileset = ./guide; };
          nativeBuildInputs = [ seedCompiler xtaskCodegenGuideBin ];
          __contentAddressed = true;
          outputHashMode = "recursive";
          outputHashAlgo = "sha256";
          buildPhase = ''
            runHook preBuild
            cdz=${seedCompiler}/bin/cdz
            mkdir -p cdzb
            cdzbs=()
            # NOTE: the .cdzb BASENAME becomes the chapter Stem in the manifest (dir slug + `file` path), so keep
            # it EXACTLY the source stem (a prefix would leak into the slug). FEED ORDER (= case dir order) is the
            # cdzbs[] append order below, independent of the basenames; chapter/playground/homepage stems don't
            # collide (CamelCase vs numeric vs HomePage). The doc TYPE is detected from each doc's root node.
            # 1) chapters (sorted glob = alphabetical stem) → dirs 0000-0350
            for sexp in src/content/chapters/*.sexp; do
              stem=$(basename "$sexp" .sexp)
              "$cdz" convert --from sexpr --to binary "$sexp" > "cdzb/$stem.cdzb"
              cdzbs+=("cdzb/$stem.cdzb")
            done
            # 2) HomePage → dir 0351
            "$cdz" convert --from sexpr --to binary src/content/HomePage.sexp > cdzb/HomePage.cdzb
            cdzbs+=("cdzb/HomePage.cdzb")
            # 3) playground examples (sorted glob = 0001-0059 numeric order) → dirs 0352-0410
            for sexp in src/playground/examples/*.sexp; do
              stem=$(basename "$sexp" .sexp)
              "$cdz" convert --from sexpr --to binary "$sexp" > "cdzb/$stem.cdzb"
              cdzbs+=("cdzb/$stem.cdzb")
            done
            [ "''${#cdzbs[@]}" -gt 0 ] || { echo "guideShred: no chapter/playground/homepage .sexp found — glob/path broke" >&2; exit 1; }
            xtask-codegen-guide --shred "$out" "$cdz" "''${cdzbs[@]}"
            runHook postBuild
          '';
          # The CLI writes the per-case dirs + manifest.json straight to $out.
          dontInstall = true;
        };
        # Verify the shred emitted the expected case population (guards a silently-empty or truncated shred —
        # the manifest's own `emitted` count must match the number of case dirs actually written).
        guideShredCheck = pkgs.runCommand "guide-shred-check" { } ''
          set -euo pipefail
          dirs=$(find ${guideShred} -mindepth 1 -maxdepth 1 -type d | wc -l)
          emitted=$(${pkgs.jq}/bin/jq -r .emitted ${guideShred}/manifest.json)
          count=$(${pkgs.jq}/bin/jq -r .count ${guideShred}/manifest.json)
          deferred=$(${pkgs.jq}/bin/jq -r .deferred ${guideShred}/manifest.json)
          echo "guide-shred: $dirs case dirs; manifest count=$count emitted=$emitted deferred=$deferred"
          # EVERY case gets a dir (deferred test-mode cases too — they carry meta.deferred=true, no program),
          # so the dir population equals the manifest's total count; emitted = count - deferred are compilable.
          [ "$dirs" -eq "$count" ] || { echo "MISMATCH: $dirs case dirs != manifest count=$count"; exit 1; }
          [ "$emitted" -eq "$((count - deferred))" ] || { echo "MISMATCH: emitted=$emitted != count-deferred=$((count - deferred))"; exit 1; }
          [ "$emitted" -gt 300 ] || { echo "too few emitted cases ($emitted) — shred likely broke"; exit 1; }
          echo "ok: guide-shred $dirs case dirs (count=$count emitted=$emitted deferred=$deferred)" > "$out"
        '';

        # ── guide-examples PER-CASE matrix (mirrors corpus mkCorpusBuild/Exec, adapted to the guide grading
        # model). Enumerate cases at EVAL from the COMMITTED guide/examples-manifest.json (v-guide-infra #5102,
        # render-independent → byte-identical to a fresh shred manifest; NO IFD — the flake bans it). The
        # program SOURCES + `expected` come from the cached `guideShred` output at BUILD time; only the case
        # LIST (dir/surfaces/graded/expectKind/peers) is read at eval. The 7 deferred test-mode cases (no
        # program) are skipped in v1 (they need the @test-export driver — a v2 shred kind).
        guideManifest = builtins.fromJSON (builtins.readFile ./guide/examples-manifest.json);
        # SKIP-SET = the manifest's own `blockedDirs` (v-guide-infra #5113) — the serial check:examples
        # known-blocked list exported AS DATA, so the sharded matrix's pass-set == the serial check's by
        # construction + self-heals when a block is added/removed (no hardcoding). Currently EMPTY (dir 0239
        # PlatformExecution's reducer bug was FIXED by #5098 record-field-name projection → the serial check
        # is 410/0), so the matrix skips nothing → fully green. (The 7 deferred test-mode cases carry no
        # program and are filtered separately.)
        guideKnownFailingDirs = guideManifest.blockedDirs or [ ];
        guideCaseList = builtins.filter
          (c: !(c.deferred or false) && !(builtins.elem c.dir guideKnownFailingDirs))
          guideManifest.cases;
        # BUILD one (case, surface): convert program.<surface> → binary AST (the front-end `cdz convert`, pure
        # syntax — cdz-compile is ast-only + cdz-wasm can't emit .ast + the guide's wrap/lower stays JS, so the
        # shred emits SOURCE and the parse lives here), then compile → emit.wasm, capturing the outcome (a
        # decline is NOT a derivation failure — the exec grades it). Multi-file: the peers (module-<name>.
        # <surface>) convert to sibling module ASTs + `--entry`. Content-addressed so a re-emit of identical
        # bytes cache-hits the exec. `seedCompiler/bin/cdz convert` is the same converter the harness runs use.
        mkGuideBuild = { dir, surface, entryName, peers }:
          pkgs.runCommand "guide-build-${dir}-${surface}"
            {
              nativeBuildInputs = [ seedCompiler cdzCompile ];
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -euo pipefail
            mkdir -p "$out"
            case=${guideShred}/${dir}
            # Forward the grade inputs up front so every early-exit path carries them.
            cp "$case/expect-kind" "$out/expect-kind"
            [ -e "$case/expected" ] && cp "$case/expected" "$out/expected" || true
            # CONVERT (parse) program.<surface> → binary AST. A PARSE error is a VALID decline outcome — many
            # guide expect-kind=error examples fail here, not at compile (e.g. two `(world …)` forms →
            # "trailing input"). So tolerate it: record a non-zero status + emit no wasm, and let the exec
            # grade expect-kind against it (a parse decline = ok for error, = FAIL for value). Without this the
            # unwrapped convert under `set -e` would KILL the derivation on any error-example that doesn't
            # parse (sharded-vs-serial discrepancy — the serial check treats a parse error as the outcome).
            if ! cdz convert --from ${surface} --to binary "$case/program.${surface}" > program.ast 2>"$out/compile.err"; then
              printf 'parse-declined' > "$out/compile.status"; exit 0
            fi
            inputs=("ast:main=program.ast")
            entry=()
            ${pkgs.lib.concatMapStringsSep "\n" (p: ''
              if ! cdz convert --from ${p.surface} --to binary "$case/module-${p.name}.${p.surface}" > "module-${p.name}.ast" 2>>"$out/compile.err"; then
                printf 'parse-declined' > "$out/compile.status"; exit 0
              fi
              inputs+=("ast:${p.name}=module-${p.name}.ast")
              entry=(--entry ${entryName})
            '') peers}
            # COMPILE. A refusal (declines/error case) is captured, NOT a derivation failure — the exec grades
            # it. emit.wasm is present only on success.
            if cdz-compile "''${inputs[@]}" "''${entry[@]}" -t wasm -o "$out/emit.wasm" 2>>"$out/compile.err"; then
              printf '0' > "$out/compile.status"
            else
              printf '%s' "$?" > "$out/compile.status"
            fi
          '';
        # EXEC one (case, surface) — grade against the guide model (compiler-free: cdzRun + the runtime store).
        #   expect-kind=value : compile+run must succeed; if the case is graded, stdout must equal `expected`.
        #   expect-kind=error : the example must DECLINE (compile failed) OR TRAP (run failed) — a clean run is
        #                       a failure. (Guide errors are authored to not-compile-or-not-run, no baseline.)
        mkGuideExec = { dir, surface, build }:
          pkgs.runCommand "guide-exec-${dir}-${surface}"
            {
              nativeBuildInputs = [ cdzRun ];
            } ''
            set -euo pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            status=$(cat ${build}/compile.status)
            ek=$(cat ${build}/expect-kind)
            if [ "$ek" = error ]; then
              if [ "$status" != 0 ]; then
                echo "ok: guide ${dir} (${surface}) — expected error, declined at compile"
              elif cdz-run ${build}/emit.wasm >/dev/null 2>trap.err; then
                echo "FAIL guide ${dir} (${surface}): expect-kind=error but compiled AND ran clean"; exit 1
              else
                echo "ok: guide ${dir} (${surface}) — expected error, trapped at run"
              fi
            else
              if [ "$status" != 0 ]; then
                echo "FAIL guide ${dir} (${surface}): expected a value but compile declined:"; cat ${build}/compile.err; exit 1
              fi
              got=$(cdz-run ${build}/emit.wasm 2>run.err) || { echo "FAIL guide ${dir} (${surface}): run trapped:"; cat run.err; exit 1; }
              if [ -e ${build}/expected ]; then
                want=$(cat ${build}/expected)
                [ "$got" = "$want" ] || { echo "FAIL guide ${dir} (${surface}): value mismatch — got [$got] want [$want]"; exit 1; }
                echo "ok: guide ${dir} (${surface}) — value [$got]"
              else
                echo "ok: guide ${dir} (${surface}) — compiled + ran clean (ungraded)"
              fi
            fi
            echo ok > "$out"
          '';
        # one exec per (case, surface) — 399 cases carry both sexpr+ml, the multi-file case sexpr-only.
        guideCaseChecks = builtins.listToAttrs (builtins.concatMap
          (c: map
            (surface: {
              name = "${c.dir}-${surface}";
              value = mkGuideExec {
                inherit (c) dir;
                inherit surface;
                build = mkGuideBuild {
                  inherit (c) dir;
                  inherit surface;
                  entryName = c.entryName or "main";
                  peers = c.peers or [ ];
                };
              };
            })
            c.surfaces)
          guideCaseList);
        # Per-FILE aggregate: force every (case,surface) exec whose case came from that chapter/source file.
        guideFileStems = pkgs.lib.unique (map (c: pkgs.lib.removeSuffix ".tsx" (baseNameOf c.file)) guideCaseList);
        mkGuideFileAgg = stem:
          let
            fileCases = builtins.filter (c: (pkgs.lib.removeSuffix ".tsx" (baseNameOf c.file)) == stem) guideCaseList;
            execs = builtins.concatMap (c: map (surface: guideCaseChecks."${c.dir}-${surface}") c.surfaces) fileCases;
          in
          assert (builtins.length execs) > 0;
          pkgs.runCommand "guide-examples-shredded-${stem}" { } ''
            ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') execs}
            echo "ok: guide-examples ${stem} — ${toString (builtins.length execs)} (case,surface) execs" > "$out"
          '';
        guideFileAggs = builtins.listToAttrs (map
          (stem: { name = "guide-examples-shredded-${stem}"; value = mkGuideFileAgg stem; })
          guideFileStems);
        # TOP aggregate: force every (case,surface) exec across all files — the sharded replacement for the
        # serial check:examples (each case cached independently; a red case fails here).
        guideExamplesShredded = pkgs.runCommand "guide-examples-shredded" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (d: ''cat ${d} > /dev/null'') (builtins.attrValues guideFileAggs)}
          echo "ok: guide-examples-shredded — ${toString (builtins.length (builtins.attrNames guideCaseChecks))} (case,surface) execs across ${toString (builtins.length guideFileStems)} files" > "$out"
        '';
        # DRIFT-GUARD: the committed manifest MUST equal a freshly-shredded manifest (render-independent, so a
        # byte-identical JSON — v-guide-infra owns regen, this makes staleness a LOUD red, not a silent skip).
        guideManifestDriftAssert = pkgs.runCommand "guide-manifest-drift-assert" { nativeBuildInputs = [ pkgs.jq ]; } ''
          set -euo pipefail
          if diff <(jq -S . ${./guide/examples-manifest.json}) <(jq -S . ${guideShred}/manifest.json) > manifest.diff; then
            echo "ok: committed guide/examples-manifest.json == fresh shred manifest" > "$out"
          else
            echo "DRIFT: committed guide/examples-manifest.json != fresh shred manifest — regen (the node shred-examples.mjs was retired for the Rust xtask-codegen-guide --shred, seq-248; the manifest IS the guideShred derivation output, so regen from it and it can't drift):"; echo "  nix build .#guide-shred && cp -f result/manifest.json guide/examples-manifest.json"
            cat manifest.diff; exit 1
          fi
        '';
        # INLINE (cdz …) RENDER GATE (v-guide-infra): every AST-backed inline Cadenza span — the codegen emits
        # `<Cadenza ast="<base64>" kind="…">` (#7245) — must RENDER in both surfaces from its embedded binary
        # AST. check:examples/guideShred gate only runnable/exercise SOURCES, so a mis-rendering INLINE prose
        # span was ungated. This is the NATIVE gate twin of the node check:cdz-render (which is advisory-tier,
        # unenforced under the sole-localGate model): it decodes each committed span's base64 + `cdz convert
        # --from binary --to {ml,sexpr}` — no compile, so cheap. Folded into localGate so a span embedding a
        # non-renderable AST reds the MERGE gate, not silently in-browser (v-guide-editor request).
        guideCdzRenderAssert = pkgs.runCommand "guide-cdz-render-assert"
          { nativeBuildInputs = [ seedCompiler pkgs.gnugrep pkgs.coreutils ]; } ''
          set -euo pipefail
          cdz=${seedCompiler}/bin/cdz
          # Gather every AST-backed inline Cadenza span's base64 from the generated chapter .tsx. Loop over a
          # FILE (not a pipe) so the failure flag survives (a piped `while read` runs in a subshell).
          grep -hoE 'ast="[A-Za-z0-9+/=]+"' ${./guide/src/content/chapters}/*.tsx | sed 's/^ast="//; s/"$//' > all.b64
          count=$(wc -l < all.b64)
          # Vacuous-pass guard: the guide has many migrated (cdz …) spans, so ~none means the emitted shape
          # changed and this extractor drifted (a silent green that would let a real render regression through).
          [ "$count" -ge 50 ] || { echo "guide-cdz-render-assert: only $count AST-backed <Cadenza ast=…> spans found — extractor drift (the emitted shape likely changed)"; exit 2; }
          fail=0
          while read -r b64; do
            printf '%s' "$b64" | base64 -d > frag.cdzb || { echo "FAIL: undecodable base64 span"; fail=1; continue; }
            for surface in ml sexpr; do
              "$cdz" convert --from binary --to "$surface" frag.cdzb > /dev/null 2>err \
                || { echo "FAIL: a (cdz …) embedded AST did not render --to $surface: $(head -c 160 err)"; fail=1; }
            done
          done < all.b64
          [ "$fail" -eq 0 ] || { echo "guide-cdz-render-assert: one or more inline (cdz …) spans failed to render"; exit 1; }
          echo "ok: $count inline (cdz …) spans render in ml+sexpr from their embedded AST" > "$out"
        '';

        # Lean 4.32.2 toolchain (v-wasm-oracle talos wasm-oracle pin) — the OFFICIAL leanprover PREBUILT
        # elan release, pinned + autoPatchelf'd. ISOLATED from the fleet nixpkgs on purpose: nixpkgs lean4 is
        # 4.30.0 on BOTH unstable + master and builds from source, and a fleet nixpkgs bump would rotate the
        # whole toolchain → REQUIRED_RUNTIME_HASH flag-day. Prebuilt is PREFERRED here: the official v4.32.2
        # release IS the reference talos + Mathlib v4.32.2 are pinned against (a source re-derivation would
        # drift). Consumed ONLY by oracle-lean (the wasm-oracle half); the fleet default gate never pulls it.
        # Co-design [[lean-432-toolchain-and-talos-mathlib-codesign]] — atomic-land with the oracle-lean
        # lean-toolchain bump to v4.32.2 (v-wasm-oracle's zone; flipping oracleLean here alone while the
        # committed lean-toolchain still says v4.30.0 mismatches, so the wiring lands together with their bump).
        lean4_432 = pkgs.stdenv.mkDerivation {
          pname = "lean4";
          version = "4.32.2";
          src = pkgs.fetchurl {
            url = "https://github.com/leanprover/lean4/releases/download/v4.32.2/lean-4.32.2-linux_aarch64.tar.zst";
            hash = "sha256-ea7n7JD3IXV9Q/dddf4uZZ3tB/C05fFWnpKsmhg+P4E=";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook pkgs.zstd ];
          buildInputs = [ (pkgs.lib.getLib pkgs.stdenv.cc.cc) pkgs.zlib pkgs.gmp ];
          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            cp -a . "$out/"
            runHook postInstall
          '';
        };

        # ── talos wasm-interpreter + its lake deps, fetched for OFFLINE lake resolution ─────────────────
        # (v-wasm-oracle talos wasm-oracle pin; co-design [[lean-432-toolchain-and-talos-mathlib-codesign]]).
        # WIP FIRST-CUT (2026-09-01, co-iterated with v-wasm-oracle on this branch): provides the hermetic dep
        # SOURCES so oracle-lean's `require talos` + Driver adapter (v-wasm-oracle's zone, added here) can build
        # OFFLINE. talos's interpreter/lake-manifest.json (lakeVersion 1.2.0) pins these 9 deps + talos@b8d8b66.
        # The adapter imports ONLY the Mathlib-FREE execution modules (Interpreter.Wasm.SmallStep + Decoder.Wat),
        # so lake RESOLVES all deps (source present) but BUILDS only the Std-only execution subgraph — NO Mathlib
        # olean build. Sources prefetched via `nix store prefetch-file` of the github archive tarballs → FILE
        # hashes → `fetchurl` + unpack (NOT fetchFromGitHub, which NAR-hashes the tree = a different hash).
        # NOTE: the exact `.lake/packages` staging + lake-manifest.json shape + the lakefile `require` gets
        # nailed empirically with v-wasm-oracle on this branch — `talosLakePackages` is the raw material for it.
        talosDepSrcs = {
          talos = { owner = "cajal-technologies"; repo = "talos"; rev = "b8d8b66602731caa38430cc39ae96e9078f56d03"; hash = "sha256-/dLm92wC/1tuVON9rxSOzkbwM90W0gvRWm6a6xgOItU="; };
          mathlib = { owner = "leanprover-community"; repo = "mathlib4"; rev = "905b95818eb3"; hash = "sha256-23Q+VjRcafMZ9Cxaa+t/4Rdcig21oYl6+IKQHsU98uU="; };
          batteries = { owner = "leanprover-community"; repo = "batteries"; rev = "023ce7d62a05"; hash = "sha256-pQZv6i/RoxHDyV1TDwcDAUbeOhk+hqcnGuCmytUkEpQ="; };
          aesop = { owner = "leanprover-community"; repo = "aesop"; rev = "a7dbf0c63b69"; hash = "sha256-uMAPl9rEbEsd1k8Hu4DAlj6mH/XdmyOUFEgad8BcWQg="; };
          Qq = { owner = "leanprover-community"; repo = "quote4"; rev = "38d591e778f1"; hash = "sha256-f1xF00eZ5hW/ZEXDt+8UohxmSEPxR2BGja/Eh37RR3I="; };
          proofwidgets = { owner = "leanprover-community"; repo = "ProofWidgets4"; rev = "6e311e2a844d"; hash = "sha256-3/tGUgA/Mfjjk+DyiHUm7EtsyCRLlgr6jcvBkYsCDGg="; };
          plausible = { owner = "leanprover-community"; repo = "plausible"; rev = "e12c1910fe85"; hash = "sha256-KCXW89f8nSYVFxCuZD4IqrsCJRvZTEdv1SF/kbWRSbM="; };
          importGraph = { owner = "leanprover-community"; repo = "import-graph"; rev = "7e9612bf0b9e"; hash = "sha256-xe6TiAv2jZrocoDHc5BGC4rz7jD3SM0Q0JjUEh7Xkww="; };
          LeanSearchClient = { owner = "leanprover-community"; repo = "LeanSearchClient"; rev = "c5d5b8fe6e51"; hash = "sha256-GobbiaaVhJzgZZAoTmuJxfI1Pjly5guJuftBJ+DP/zE="; };
          Cli = { owner = "leanprover"; repo = "lean4-cli"; rev = "88679d088c97"; hash = "sha256-4qHZ18NBvG1vlnSUDebBKC67L7ZopnfIQFxcADnT2yY="; };
        };
        # Unpack each dep tarball into `$out/<name>/` (the repo root — the shape lake's `.lake/packages/<name>`
        # expects). v-wasm-oracle's lakefile `require` + a generated lake-manifest.json (added here) wire lake to
        # resolve against these offline.
        talosLakePackages = pkgs.runCommand "talos-lake-packages" { } (
          pkgs.lib.concatStringsSep "\n" (pkgs.lib.mapAttrsToList
            (name: d:
              let src = pkgs.fetchurl {
                    url = "https://github.com/${d.owner}/${d.repo}/archive/${d.rev}.tar.gz";
                    inherit (d) hash;
                  };
              in ''
                mkdir -p "$out/${name}"
                tar -xzf ${src} -C "$out/${name}" --strip-components=1
              '')
            talosDepSrcs));

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
            # the offline lake-manifest (v-wasm-oracle talos pin): pins Interpreter + its 9 deps as path
            # entries so lake resolves against the pre-staged .lake/packages (below) with no network.
            ./implementation/oracle-lean/lake-manifest.json
            ./implementation/oracle-lean/Oracle.lean
            ./implementation/oracle-lean/Main.lean
            ./implementation/oracle-lean/OracleTest.lean
            ./implementation/oracle-lean/OracleAstTest.lean
            ./implementation/oracle-lean/OracleCheck.lean
            # the wasm-differential exe root (talos-driven Core↔wasm conformance runner). MUST be enumerated
            # or `lake build oracle-wasm-diff` fails "no such file" — and, since it is not in `src`, an edit
            # to ONLY this file leaves the drv hash unchanged so nix serves a STALE cached build (its
            # native_decide e2e witnesses silently never recompile). #7335 added the exe to the build/install
            # lines but omitted it here; that gap is closed now (its absence made #7343's e2e gate hollow).
            ./implementation/oracle-lean/OracleWasmDiffTest.lean
            ./implementation/oracle-lean/Oracle
          ];
        };
        oracleLean = pkgs.stdenv.mkDerivation {
          pname = "cdz-oracle-lean";
          version = "0.0.0";
          src = oracleLeanSrc;
          # lean4_432 (the pinned 4.32.2 toolchain) — the talos wasm-oracle needs Lean 4.32.2. Flipped from
          # pkgs.lean4 (4.30.0) as part of the talos co-land: this MUST land atomically with the oracle-lean
          # lean-toolchain bump to v4.32.2 (v-wasm-oracle's zone, added on this branch) — flipping alone while
          # the committed lean-toolchain still says v4.30.0 mismatches. WIP on this co-iteration branch.
          # lean4_432 is the PREBUILT elan release → its leanc links the produced exes against the FHS
          # interpreter /lib/ld-linux-aarch64.so.1, which is ABSENT in the pure nix sandbox → "required file
          # not found" at runtime (dev-shell masks it since the host has /lib/ld-linux). v-nix's fix, applied
          # here as part of the talos co-land: autoPatchelfHook rewrites the exes' interpreter → the nix ld +
          # rpath; buildInputs supply the runtime libs (libleanshared via lean4_432, + libcc/gmp/zlib);
          # appendRunpaths add lean's shared-lib dirs so the patched exes resolve libleanshared.
          nativeBuildInputs = [ lean4_432 pkgs.autoPatchelfHook ];
          buildInputs = [ lean4_432 pkgs.gmp pkgs.zlib (pkgs.lib.getLib pkgs.stdenv.cc.cc) ];
          appendRunpaths = [ "${lean4_432}/lib" "${lean4_432}/lib/lean" ];
          buildPhase = ''
            runHook preBuild
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            # fileset.toSource copies are read-only; lake writes .lake/ into the tree.
            chmod -R u+w .
            # Stage talos + its 9 lake deps into .lake/packages WRITABLE for OFFLINE lake resolution
            # (v-wasm-oracle validated: lake writes .lake/ metadata into every manifest dep incl. unbuilt
            # mathlib, so read-only store symlinks EACCES; git-type manifest entries trigger re-clone → the
            # committed lake-manifest.json uses path-type; talos's package is at interpreter/ (name
            # "Interpreter") so stage THAT as .lake/packages/talos). Mathlib-free: lake builds only the
            # imported exec closure (SmallStep+Decoder.Wat, Std-only) → mathlib fetched but 0 oleans built.
            mkdir -p .lake/packages
            cp -r ${talosLakePackages}/talos/interpreter .lake/packages/talos
            for d in mathlib batteries aesop Qq proofwidgets plausible importGraph LeanSearchClient Cli; do
              cp -r ${talosLakePackages}/$d .lake/packages/$d
            done
            chmod -R u+w .lake/packages
            lake build cdz-oracle oracle-selftest oracle-ast-roundtrip oracle-check oracle-wasm-diff
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p "$out/bin"
            install -m755 .lake/build/bin/cdz-oracle "$out/bin/cdz-oracle"
            install -m755 .lake/build/bin/oracle-selftest "$out/bin/oracle-selftest"
            install -m755 .lake/build/bin/oracle-ast-roundtrip "$out/bin/oracle-ast-roundtrip"
            install -m755 .lake/build/bin/oracle-check "$out/bin/oracle-check"
            install -m755 .lake/build/bin/oracle-wasm-diff "$out/bin/oracle-wasm-diff"
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

        # ── wasm-oracle emit-extraction harness (v-wasm-oracle #3) ──────────────────────────────────────────
        # Per corpus case, produce the triple v-lean-oracle's `oracle-wasm-diff` runner consumes:
        #   core.wat        — `wasm-tools print` of the unbundled core module (the emitted wasm the differential
        #                     interprets via talos), from mkCorpusBuild's `emit.wasm` (cached component)
        #   result-type.ast — raw `@custom "cdz-result-type"` section bytes (rtBytes for resultScalarTy?)
        #   core.ast        — the OPTIMIZED Core binary-AST (`-t cadenza`) that `reduce` runs (the Core reference)
        # A case that did NOT compile to a component (declined/errored → no emit.wasm), has no unbundlable core
        # module, or no cdz-result-type section → EMPTY $out (the manifest lists only dirs with a core.wat, so
        # such cases are simply absent → the runner never sees them). Import-bearing (heap) core modules DO get a
        # core.wat but talos declines them at run → differential `.skip` (sound gap) until the W5+ runtime host.
        mkWasmExtract = { name, shred, build, idx }:
          pkgs.runCommand "wasm-extract-${name}-${idx}"
            {
              nativeBuildInputs = [ pkgs.wasm-tools cdzCompile ];
              # Content-addressed (v-nix flake review): each extraction caches on {emit.wasm bytes + wasm-tools},
              # so a compiler-rev bump that re-emits identical bytes reuses the extraction. Mirrors mkCorpusBuild.
              __contentAddressed = true;
              outputHashMode = "recursive";
              outputHashAlgo = "sha256";
            } ''
            set -uo pipefail
            mkdir -p "$out"
            [ -e "${build}/emit.wasm" ] || exit 0
            case=$(echo ${shred}/${name}/${idx}-*)
            [ -d "$case" ] || exit 0
            mkdir -p cores
            wasm-tools component unbundle "${build}/emit.wasm" --threshold 0 --module-dir cores -o /dev/null 2>/dev/null || { rm -rf "$out"/* ; exit 0; }
            [ -e cores/unbundled-module0.wasm ] || { rm -rf "$out"/* ; exit 0; }
            wasm-tools print cores/unbundled-module0.wasm > "$out/core.wat" 2>/dev/null || { rm -rf "$out"/* ; exit 0; }
            cdz-compile "ast:main=$case/program.ast" -t cadenza -o "$out/core.ast" 2>/dev/null || { rm -rf "$out"/* ; exit 0; }
            # result-type.ast: the cdz-result-type custom-section byte range (objdump), sliced from emit.wasm.
            # The range points directly at the `cdzast…` blob (no name prefix — v-wasm-oracle verified).
            off=$(wasm-tools objdump "${build}/emit.wasm" 2>/dev/null | grep 'cdz-result-type' | grep -oE '0x[0-9a-f]+' | head -2)
            s=$(echo "$off" | sed -n 1p); e=$(echo "$off" | sed -n 2p)
            { [ -n "$s" ] && [ -n "$e" ]; } || { rm -rf "$out"/* ; exit 0; }
            dd if="${build}/emit.wasm" of="$out/result-type.ast" bs=1 skip=$((s)) count=$(( $((e)) - $((s)) )) status=none 2>/dev/null
          '';
        # Per corpus file → the list of per-case extraction dirs (empty for skipped cases). STEP A (uncapped):
        # every case of the scoped scalar files. The FIRST-PROOF CAP is gone — the per-case emit.wasm builds are
        # now cache-warm on cachix (corpus-emit-wasm-warm → cache-warm-emit-wasm.yml), so mkCorpusBuild is pulled,
        # not recompiled. The mkWasmExtract layer (unbundle/print + cdz-compile -t cadenza + objdump/dd) still
        # runs cold on the first full build (CA-cached after); v-lean-oracle chose momentum over pre-warming it,
        # so the first uncapped run goes via `with-lease`. STEP B (later): widen `wasmOracleFiles` to the whole
        # corpus once Step A is proven green at ~1478 cases.
        wasmExtractFileDirs = { name, file }:
          let
            shred = mkCorpusShred { inherit name file; };
            n = corpusCaseCount file;
            idxs = builtins.genList (i: pkgs.lib.fixedWidthNumber 4 i) n;
          in map (idx: mkWasmExtract { inherit name shred idx; build = mkCorpusBuild { inherit name shred idx; }; }) idxs;
        # SCOPE (Step A): scalar-heavy corpus files so the initial differential run is tractable; widen to
        # `corpusFileNames` once the pipeline is proven green over these.
        wasmOracleFiles = [ "01-literals.sexp" "06-numeric-model.sexp" ];
        # All per-case extraction dirs for the scoped files — shared by the manifest (oracleWasmCaseDirs) and the
        # extraction cache-warm (corpusWasmExtractWarm), so both realize exactly the same set.
        wasmOracleExtractDirs = pkgs.lib.concatLists (map
          (f: let stem = pkgs.lib.removeSuffix ".sexp" f; in
            wasmExtractFileDirs { name = stem; file = ./spec/semantics + "/${f}"; })
          wasmOracleFiles);
        # The manifest v-lean-oracle's oracle-wasm-diff check reads: one line per per-case dir that HAS a
        # core.wat (a runnable extraction), sorted. Mirrors oracleLeanCaseDirs. The dir list is passed via
        # `passAsFile` (NOT an env attr): at ~1478 dirs (Step A) it's ~90KB and at full-corpus Step B ~640KB —
        # a runCommand env attr stringifies the list into one env var that counts against the execve arg+env
        # limit (the E2BIG that bit corpus-emit-wasm-warm, #7546), so read it from a file instead.
        oracleWasmCaseDirs = pkgs.runCommand "oracle-wasm-case-dirs"
          {
            dirs = wasmOracleExtractDirs;
            passAsFile = [ "dirs" ];
          } ''
          : > "$out"
          for d in $(tr ' ' '\n' < "$dirsPath"); do [ -e "$d/core.wat" ] && echo "$d" >> "$out" || true; done
          sort -o "$out" "$out"
          echo "oracle-wasm-case-dirs: $(wc -l < "$out") runnable extractions" >&2
        '';
        # Extraction-layer cache-warm: realize every per-case wasm extraction (mkWasmExtract: unbundle/print +
        # `cdz compile -t cadenza` + objdump/dd) so v-gha-green can push them to cachix and v-lean-oracle's
        # oracle-wasm-diff check pulls them instead of cold-building (~12h cold under a vertical lease). The
        # emit.wasm layer is already warmed by corpus-emit-wasm-warm; this is the layer ON TOP of it. Same
        # writeText-manifest pattern as corpus-emit-wasm-warm (a single store path, NOT an env attr) so the
        # ~1478 (Step A) / ~10.7k (Step B) extraction paths never hit the execve arg+env limit.
        corpusWasmExtractWarm = pkgs.runCommand "corpus-wasm-extract-warm" { } ''
          cp ${pkgs.writeText "corpus-wasm-extract-dirs"
            (pkgs.lib.concatStringsSep "\n" wasmOracleExtractDirs)} "$out"
          echo "corpus-wasm-extract-warm: realized $(wc -l < "$out") per-case wasm extractions" >&2
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
        # Cross-shell PATH wrapper-scripts for the all-nix entrypoints (v-nix 2026-08-28). Hoisted here so
        # BOTH devShells.default (packages) AND packages.cdz-shell-wrappers use the SAME wrappers (no drift).
        # NOT shell functions: agents' claude Bash-tool subshells are ZSH + the shell snapshot HARD-RESETS
        # PATH per command (v-ft), so functions/nix-develop-PATH never reach them. Delivery: v-ft symlinks
        # packages.cdz-shell-wrappers/bin/* into ~/.local/bin (@snapshot-PATH pos49, writable, BEFORE
        # rustup ~/.cargo/bin) so they resolve in every agent subshell + a cargo-shim there shadows cargo.
        # Each execs `nix run <worktree>#app` → rebuild-on-edit from the dirty worktree (needs only nix+git,
        # which the snapshot PATH has) — a fixed wrapper forwarding to a rebuilding target, not a frozen bin.
        cdzShellWrap = name: app: pkgs.writeShellApplication {
          inherit name;
          runtimeInputs = [ pkgs.nix pkgs.git ];
          text = ''
            root="$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")"
            exec nix run --option warn-dirty false "$root#${app}" -- "$@"
          '';
        };
        cdzShellHelp = pkgs.writeShellApplication {
          name = "cdz-help";
          runtimeInputs = [ ];
          text = ''
            cat <<'CDZHELP'
            cdz all-nix shell — custom commands (nix compiles on demand from your worktree, warm-cached):
              cdz …               compile / run / test / doctor  (builds the component store on 1st run)
              cdz-run FILE.wasm   run a component
              cdz-compile …       the standalone compiler (what cdz delegates to)
              roundtrip [files]   corpus round-trip (sexpr exact-repro + ml fixed-point)
              lint-mandates       the mandate lint (no-integration-tests etc.; replaces cargo xtask lint-mandates)
              fast-gate [crates]  fast touched-crate gate (inner loop)
              gate                full local-gate battery (convenience)
              cdz-help            print this list
              → authoritative MERGE gate stays: cargo xtask fleet gate-local
            CDZHELP
          '';
        };
        cdzShellWrappers = [
          (cdzShellWrap "cdz" "cdz")
          (cdzShellWrap "cdz-run" "cdz-run")
          (cdzShellWrap "cdz-compile" "cdz-compile")
          (cdzShellWrap "roundtrip" "roundtrip")
          (cdzShellWrap "lint-mandates" "lint-mandates")
          (cdzShellWrap "gate" "gate")
          (cdzShellWrap "fast-gate" "fast-gate")
          cdzShellHelp
        ];
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
        # The rc-trace runtime variant (debug-counters + rc-trace-export, world runtime-debug) — cdz-run
        # --rc-trace consumes it by explicit --runtime; v-corpus-harness wires that. `nix build .#rctrace-runtime`.
        packages.rctrace-runtime = runtimeRctrace;
        # The `*-hash` outputs are the SHARED `hashOf` derivations (also consumed by componentStore + the
        # compiler-hash injection + the NFC-stamp), so `nix build .#runtime-hash` yields the exact file those
        # consumers `cat` — one hash derivation per component, not one per use-site.
        packages.runtime-hash = runtimeHash;
        packages.runtime-debug-hash = runtimeDebugHash;
        packages.rctrace-runtime-hash = runtimeRctraceHash;

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
        # packages.cdz-shell-wrappers — the 7 all-nix entrypoint PATH wrappers (cdz/cdz-run/cdz-compile/
        # roundtrip/gate/fast-gate/cdz-help) as a single symlinkJoin, so v-fleet-tooling's window.sh can
        # `ln -sf result/bin/* ~/.local/bin` (a snapshot-PATH dir) each boot — reaching the agents' zsh
        # Bash-tool subshells, which the shell snapshot's PATH hard-reset otherwise cuts off from nix
        # develop. Same wrappers the devShell ships (shared cdzShellWrappers, no drift).
        packages.cdz-shell-wrappers = pkgs.symlinkJoin {
          name = "cdz-shell-wrappers";
          paths = cdzShellWrappers;
        };

        # xtask dev-tool binary as a relocatable nix package (v-xtask-decompose). `nix build .#xtask` →
        # result/bin/xtask. The per-subcommand `apps.*` (roundtrip, &c.) wrap it; a direct `nix run .#xtask
        # -- <cmd>` works too if `CDZ_REPO_ROOT` is set (the apps set it for you).
        packages.xtask = xtaskBin;

        # The standalone roundtrip command bin. `nix build .#xtask-roundtrip` → result/bin/xtask-roundtrip.
        packages.xtask-roundtrip = xtaskRoundtripBin;

        # The standalone emoji-ban lint bin (v-xtask-decompose). `nix build .#xtask-lint-emoji` →
        # result/bin/xtask-lint-emoji. Backs `apps.lint-emoji`; caches independently of xtask.
        packages.xtask-lint-emoji = xtaskLintEmojiBin;

        # The standalone baseline canonicalizer bin (v-xtask-decompose). `nix build
        # .#xtask-canonicalize-baselines` → result/bin/xtask-canonicalize-baselines. Backs
        # `apps.canonicalize-baselines`; caches independently of xtask.
        packages.xtask-canonicalize-baselines = xtaskCanonicalizeBaselinesBin;

        # The standalone Cadenza formatter bin (v-xtask-decompose). `nix build .#xtask-fmt` →
        # result/bin/xtask-fmt. Backs `apps.fmt`; caches independently of xtask.
        packages.xtask-fmt = xtaskFmtBin;

        # The WIT-world artifact utility bin (v-xtask-decompose). `nix build .#world-artifact` →
        # result/bin/cdz-world-artifact. Backs `apps.world-artifact` (the `cargo xtask world-artifact`
        # replacement). Distinct from `packages.world-artifacts` (plural = the emitted-artifacts derivation).
        packages.world-artifact = cdzWorldArtifactBin;

        # The contract-schema projector bin (v-xtask-decompose, codegen→build-time-nix). `nix build
        # .#xtask-codegen-contracts` → result/bin/xtask-codegen-contracts. The `cdzPlatformContracts`
        # derivation (v-nix, in progress) runs it to emit cdz-platform/src/contracts/*.rs at build time.
        packages.xtask-codegen-contracts = xtaskCodegenContractsBin;
        # The build-time-generated contract schemas (v-nix). `nix build .#cdz-platform-contracts` →
        # result/contracts/{<name>.rs,mod.rs}. The atomic overlay-flip will stage these into cdz-platform's
        # compile in place of the (to-be-dropped) committed src/contracts.
        packages.cdz-platform-contracts = cdzPlatformContracts;
        # The build-time-generated wasm ABI byte-table (v-nix). `nix build .#cdz-wasm-abi` → result/wasm_abi.rs.
        packages.cdz-wasm-abi = cdzWasmAbi;

        # The wasm/component byte-table extractor bin (v-xtask-decompose, codegen→build-time-nix). `nix build
        # .#xtask-codegen-wasm-abi` → result/bin/xtask-codegen-wasm-abi. A `cdzWasmAbi` derivation runs it to
        # emit rcdzc/src/backend/wasm/wasm_abi.rs at build time.
        packages.xtask-codegen-wasm-abi = xtaskCodegenWasmAbiBin;

        # The guide sexp→TSX codegen bin (v-guide-infra I5; v-nix nix wiring). `nix build .#xtask-codegen-guide`
        # → result/bin/xtask-codegen-guide. guideExamplesCheck sets CDZ_XTASK_CODEGEN_GUIDE to this so the
        # guide's `npm run codegen` regenerates the @generated .tsx from the .sexp source-of-truth in-gate.
        packages.xtask-codegen-guide = xtaskCodegenGuideBin;

        # The DEPLOYABLE guide static site (guide/dist/) built through nix (operator 2026-08-29: nixify the
        # Pages deploy for the shared cache). `nix build .#guide-site` → result/ = the site the pages.yml
        # deploy uploads as the Pages artifact (cache-hit on unchanged trunk instead of a cold ARM rebuild).
        packages.guide-site = guideSite;

        # `.#corpus-verdicts` — the WASM-corpus verdict harvest (v-xtask-decompose --save gate-delete). One
        # `<tag>\t<description>` line per case, concatenated across the whole corpus. `apps.save-baseline`
        # (pending v-xtask's xtask-save-baseline leaf on main) feeds this to the leaf to regenerate .gate-baseline.
        packages.corpus-verdicts = corpusVerdictsAll;

        # `.#corpus-verdicts-coarse` — the storm-free COARSE harvest (~35 file derivations, one per file, each
        # compiling+grading its cases internally) that will REPLACE corpusVerdictsAll as apps.save-baseline's
        # source once v-corpus-harness signs off parity. `.#corpus-verdicts-coarse-parity` is the per-file
        # byte-identity spike (coarse == per-case verdictsFileAgg). See mkCorpusVerdictsFileCoarse's def note.
        packages.corpus-verdicts-coarse = corpusVerdictsCoarseAll;
        packages.corpus-verdicts-coarse-parity = corpusVerdictsCoarseParity;
        # DIVERSE-SAMPLE per-file parity packages (v-corpus-harness acceptance step 2): distinct case shapes —
        # 05-compound-types (value-heavy #record/#tuple), 11-modules (multi-module → --entry main),
        # 25-verification (big, cross-module type-import), 26-program-conditions (traps/@invariant/diagnostics),
        # 29-cross-component-peers (--peer/L3). 01-literals is corpus-verdicts-coarse-parity above.
        packages.corpus-verdicts-coarse-parity-05-compound-types = mkCoarseParity { name = "05-compound-types"; file = ./spec/semantics/05-compound-types.sexp; };
        packages.corpus-verdicts-coarse-parity-11-modules = mkCoarseParity { name = "11-modules"; file = ./spec/semantics/11-modules.sexp; };
        packages.corpus-verdicts-coarse-parity-25-verification = mkCoarseParity { name = "25-verification"; file = ./spec/semantics/25-verification.sexp; };
        packages.corpus-verdicts-coarse-parity-26-program-conditions = mkCoarseParity { name = "26-program-conditions"; file = ./spec/semantics/26-program-conditions.sexp; };
        packages.corpus-verdicts-coarse-parity-29-cross-component-peers = mkCoarseParity { name = "29-cross-component-peers"; file = ./spec/semantics/29-cross-component-peers.sexp; };
        # rust + rust-async coarse harvests + per-file parity spikes (the 3-backend set for v-corpus-harness).
        packages.corpus-verdicts-rust-coarse = corpusRustVerdictsCoarseAll;
        packages.corpus-verdicts-rust-async-coarse = corpusRustAsyncVerdictsCoarseAll;
        packages.corpus-verdicts-rust-coarse-parity = corpusRustVerdictsCoarseParity;
        packages.corpus-verdicts-rust-async-coarse-parity = corpusRustAsyncVerdictsCoarseParity;
        # DIVERSE-SAMPLE rust + rust-async per-file parity (v-corpus-harness: 01-literals scalar-only insufficient
        # after the -t rust-async variant bug — exercise multi-module + value-heavy shapes at least).
        packages.corpus-verdicts-rust-coarse-parity-11-modules = mkRustCoarseParity { name = "11-modules"; file = ./spec/semantics/11-modules.sexp; };
        packages.corpus-verdicts-rust-coarse-parity-05-compound-types = mkRustCoarseParity { name = "05-compound-types"; file = ./spec/semantics/05-compound-types.sexp; };
        packages.corpus-verdicts-rust-coarse-parity-25-verification = mkRustCoarseParity { name = "25-verification"; file = ./spec/semantics/25-verification.sexp; };
        packages.corpus-verdicts-rust-coarse-parity-26-program-conditions = mkRustCoarseParity { name = "26-program-conditions"; file = ./spec/semantics/26-program-conditions.sexp; };
        packages.corpus-verdicts-rust-coarse-parity-29-cross-component-peers = mkRustCoarseParity { name = "29-cross-component-peers"; file = ./spec/semantics/29-cross-component-peers.sexp; };
        packages.corpus-verdicts-rust-async-coarse-parity-11-modules = mkRustCoarseParity { name = "11-modules"; file = ./spec/semantics/11-modules.sexp; async = true; };
        packages.corpus-verdicts-rust-async-coarse-parity-05-compound-types = mkRustCoarseParity { name = "05-compound-types"; file = ./spec/semantics/05-compound-types.sexp; async = true; };
        packages.corpus-verdicts-rust-async-coarse-parity-25-verification = mkRustCoarseParity { name = "25-verification"; file = ./spec/semantics/25-verification.sexp; async = true; };
        packages.corpus-verdicts-rust-async-coarse-parity-26-program-conditions = mkRustCoarseParity { name = "26-program-conditions"; file = ./spec/semantics/26-program-conditions.sexp; async = true; };
        packages.corpus-verdicts-rust-async-coarse-parity-29-cross-component-peers = mkRustCoarseParity { name = "29-cross-component-peers"; file = ./spec/semantics/29-cross-component-peers.sexp; async = true; };

        # `.#corpus-verdicts-rust` / `.#corpus-verdicts-rust-async` — the RUST + RUST-ASYNC verdict harvests
        # (v-xtask-decompose, the flake.nix:3514 follow-up). Same `<tag>\t<description>` shape as the wasm
        # harvest, via `cdz-rust-run --emit-verdict` (classify-not-compare, exit 0). PACKAGES only — deliberately
        # NOT in the localGate merge-required fail-set: a verdict harvest is an `apps.save-baseline` regenerator
        # INPUT, not a gate (mirrors quoteCorpusVerdictsAll being packages-only). The rust/rust-async gate
        # stays the existing corpus-rust / corpus-rust-async checks; these are the re-baseline source.
        packages.corpus-verdicts-rust = corpusRustVerdictsAll;
        packages.corpus-verdicts-rust-async = corpusRustAsyncVerdictsAll;

        # `.#corpus-verdicts-rust-smoke` / `-rust-async-smoke` — a FAST single-file end-to-end smoke of the
        # rust/rust-async verdict harvest: the per-file agg over ONE small corpus file (30-type-ast-reflection,
        # ~2 cases) so `cdz-rust-run --emit-verdict` is exercised end-to-end in the sandbox in SECONDS, vs the
        # whole-corpus `.#corpus-verdicts-rust` (hundreds of rustc compiles, too heavy for an interactive
        # confirm). packages-only (never gated); a permanent quick smoke for the harvest + the --emit-verdict flag.
        packages.corpus-verdicts-rust-smoke = verdictsRustFileAgg {
          name = "30-type-ast-reflection";
          file = ./spec/semantics/30-type-ast-reflection.sexp;
        };
        packages.corpus-verdicts-rust-async-smoke = verdictsRustAsyncFileAgg {
          name = "30-type-ast-reflection";
          file = ./spec/semantics/30-type-ast-reflection.sexp;
        };

        # `.#quote-corpus-verdicts` — the quote-corpus round-trip verdict harvest (inc-4; mirrors
        # `.#corpus-verdicts`). `<tag>\t<description>` per eligible case (declined→todo, round-trip-ok→pass,
        # fail→fail); the regenerator input a `save` writes `.quote-gate-baseline` from (single-component
        # eligibility → module/peer cases absent by design). 🚨 reject a fail-SPIKE (starvation) before baking.
        packages.quote-corpus-verdicts = quoteCorpusVerdictsAll;

        # The standalone baseline pruner bin (v-xtask-decompose). `nix build .#xtask-prune-baselines` →
        # result/bin/xtask-prune-baselines. Backs `apps.prune-baselines`; caches independently of xtask.
        packages.xtask-prune-baselines = xtaskPruneBaselinesBin;

        # The standalone runtime allocation benchmark bin (v-xtask-decompose). `nix build .#xtask-bench` →
        # result/bin/xtask-bench. Backs `apps.bench` + the rewired `benchCheck`; caches independently of xtask.
        packages.xtask-bench = xtaskBenchBin;

        # The standalone install-lsp bin (v-xtask-decompose). `nix build .#xtask-install-lsp`. Backs
        # `apps.install-lsp`; caches independently of xtask.
        packages.xtask-install-lsp = xtaskInstallLspBin;
        packages.xtask-duvet-check = xtaskDuvetCheckBin;

        # The standalone mandate-lint binary (v-xtask-decompose). `nix build .#xtask-mandates` →
        # result/bin/xtask-mandates. Backs `apps.lint-mandates` + the mandate gate; caches independently
        # of xtask (its closure is just the crate + syn).
        packages.xtask-mandates = xtaskMandatesBin;

        # oracle-lean (L0.1): the Lean differential oracle. `nix build .#oracle-lean` →
        # result/bin/{cdz-oracle,oracle-selftest}.
        packages.oracle-lean = oracleLean;

        # rcdzc→wasm: the compiler as a wasm artifact for the agent kernel's blob store. `.#rcdzc-wasm`
        # is the wasm module; `.#rcdzc-wasm-hash` its derived content address (for v-agent-harness's
        # compiler-latest store pointer).
        packages.cdz-wasm-pkg = cdzWasmPkg;
        # The isolated Lean 4.32.2 toolchain (v-wasm-oracle talos pin) — buildable standalone for validation.
        packages.lean4-432 = lean4_432;
        # The prebuilt rust-exec rlib dir (cdz-rt/cdz-num/cadenza-ast + their deps/) — exposed so the rust
        # corpus-exec rlib set is inspectable (e.g. confirm unicode_normalization is present for the NFC cases).
        packages.rust-rlibs = rustRlibs;
        # talos + its 9 lake deps unpacked (raw material for oracle-lean's offline .lake/packages) — buildable
        # standalone so v-wasm-oracle can inspect the layout while wiring the lakefile require + manifest.
        packages.talos-lake-packages = talosLakePackages;
        # Realize every per-case corpus emit.wasm so a CI cache-warm run pushes them to cachix (v-wasm-oracle;
        # v-nix wires `.#packages.<sys>.corpus-emit-wasm-warm` into cache-warm.yml). Warms the wasm-oracle
        # emit-extraction harness's mkCorpusBuild reuse → the uncapped full-corpus Core↔wasm differential.
        packages.corpus-emit-wasm-warm = corpusEmitWasmWarm;
        # The wasm-oracle emit-extraction MANIFEST (v-wasm-oracle #3): newline-separated per-case dir paths,
        # each dir = {core.wat, result-type.ast, core.ast}. v-lean-oracle's oracle-wasm-diff check reads this
        # to run the Core↔wasm differential over the corpus. `nix build .#oracle-wasm-case-dirs`.
        packages.oracle-wasm-case-dirs = oracleWasmCaseDirs;
        # Extraction-layer cache-warm target (v-gha-green wires into a daily/dispatch workflow, like
        # cache-warm-emit-wasm.yml): realizes every per-case wasm extraction so they land on cachix.
        packages.corpus-wasm-extract-warm = corpusWasmExtractWarm;
        # The per-example shred artifact dirs (v-guide-infra CLI, v-nix wiring). `nix build .#guide-shred`.
        packages.guide-shred = guideShred;
        # The standalone calc/repl binary `cdz calc`/`cdz repl` forwards to (v-cdz-crate-split #5167). Exposed
        # so `nix build .#cdz-calc` builds it directly; apps.cdz injects it as CDZ_CALC_BIN for interactive use.
        packages.cdz-calc = cdzCalc;
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
        # `xtask codegen` already recorded. This reads the committed value only to COMPARE — the flake
        # never uses it as the build's asserted output. It catches a divergence between the nix build and
        # the xtask build (e.g. a toolchain/vendor drift) at `nix flake check` time. The three hash consts
        # are `@generated by cargo xtask codegen` into `cadenza-compile-abi/src/runtime_hash.rs` (relocated
        # there so the thin `!standalone` `cdz` reads them without linking `rcdzc`; `rcdzc`'s `runtime_abi.rs`
        # re-exports them). Each is `= match option_env!("CDZ_…") { Some(h) => h, None => "<hash>" }` (the
        # compile-time override, so a nix build can inject the runtime it built — see `seedCompiler`), so the
        # COMMITTED value is the 45-char base62 literal in the `None =>` DEFAULT arm. We split on that arm
        # marker and take the leading 45 base62 chars (guarded: the marker MUST be present and the chars MUST
        # be base62, else we THROW rather than compare against a stray literal). Platform content address, §8.
        checks =
          let
            # The 3 hash consts RELOCATED rcdzc/backend/wasm/runtime_abi.rs → cadenza-compile-abi (#6104,
            # v-runtime/v-cdz-crate-split, unblocks the rcdzc-optional flip; runtime_abi.rs now RE-EXPORTS them,
            # which is NOT a `pub const X = match{…}` decl this regex can read). runtime_hash.rs PRESERVES the
            # exact `pub const X: &str = match option_env!(…) { Some(h)=>h, None=>"<45 base62>" }` shape, so the
            # decl/marker split below still works — only the readFile PATH moves (the atomic flake half of #6104).
            abi = builtins.readFile
              ./implementation/seed/crates/cadenza-compile-abi/src/runtime_hash.rs;
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
                throw "flake.nix: `${decl}` not found in cadenza-compile-abi/src/runtime_hash.rs (codegen shape changed?)"
              else if !valid then
                throw "flake.nix: `${decl}` found but its `None =>` arm holds no 45-char base62 default literal"
              else hash;
            parity = { name, hashFile, constName }:
              pkgs.runCommand "${name}-hash-parity" { } ''
                got=$(cat ${hashFile})
                want=${recordedHash constName}
                if [ "$got" != "$want" ]; then
                  echo "PARITY FAIL: nix-built ${name} hash $got != runtime_hash.rs ${constName} $want" >&2
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
              # crateCdzCheckSrc = seedTestSrc MINUS spec/semantics (caching fast-path — see its binding note):
              # a corpus edit no longer rotates this whole-workspace check. compiler-ml + all crate src + xtask
              # stay (cdz's run_ml_cli/run_rust_cli tests + the workspace build need them).
              src = crateCdzCheckSrc;
              extraInputs = [ pkgs.git ];
            };
            # cdz-default-features check (v-nix, gate-hardening for the default-only-arm class — v-ft-agreed,
            # flake-only wiring 2026-08-29). WHY: the nix seedCompiler + the per-crate crane checks build cdz
            # `--no-default-features` (corpus-over-trigger, flake ~232), so a compile error in a DEFAULT-only
            # `#[cfg(feature=…)]` arm (watch/lsp/completions/corpus/standalone) escapes them — that is how #5259
            # (WatchCmd::Run) landed green yet broke every default/dev/packaged cdz build (the #5258→#5266
            # whack-a-mole). This is a CHEAP `cargo check -p cdz` with DEFAULT features ON (NOT
            # --no-default-features), consuming the shared cargoArtifacts (deps warm), scoped to cdz's default
            # dep-closure via craneCrateCommon (so a corpus / compiler-ml / unrelated-crate edit does NOT rotate
            # it). check-only = no codegen/link, seconds warm. Deliberately NOT in localGate: crateCdzCheck's
            # `cargo build --workspace` already compiles cdz default-features at the authoritative gate, so this
            # adds ZERO localGate weight — it exists so fast-gate (the inner loop, which has no per-crate cdz
            # check) catches the class cheaply on a touched cdz. Wired into crateChecks "cdz" below.
            cdzDefaultFeaturesCheck = craneLib.mkCargoDerivation ((craneCrateCommon { crate = "cdz"; }) // {
              pname = "cdz-default-features-check";
              doInstallCargoArtifacts = false;
              buildPhaseCargoCommand = "cargo check --locked -p cdz";
              installPhaseCommand = ''echo "ok: cdz-default-features-check (cargo check -p cdz, DEFAULT features — watch/lsp/completions/corpus/standalone arms)" > "$out"'';
            });
            # crane MR2: the CLIPPY half via crane (per-crate cargoClippy consuming the shared cargoArtifacts →
            # deps NOT recompiled each run). Each maker takes crate/extraSrc/extraInputs. cdz stays
            # workspace-src (crateCdzCheck, different shape — its clippy is inside cargoWorkspaceCheck).
            perCrateClippyCrane = {
              clippy-cadenza-ast = mkCrateClippyCrane { crate = "cadenza-ast"; };
              clippy-cadenza-compile-abi = mkCrateClippyCrane { crate = "cadenza-compile-abi"; };
              clippy-cadenza-syntax = mkCrateClippyCrane { crate = "cadenza-syntax"; extraSrc = [ ./spec/semantics ]; };
              # cadenza-syntax split (#5076/#5082): leaf surface crates, no tests/ dir, no spec/semantics dep.
              clippy-cadenza-syntax-cedar = mkCrateClippyCrane { crate = "cadenza-syntax-cedar"; };
              clippy-cadenza-syntax-core = mkCrateClippyCrane { crate = "cadenza-syntax-core"; };
              clippy-cadenza-syntax-json = mkCrateClippyCrane { crate = "cadenza-syntax-json"; };
              clippy-cadenza-syntax-sexpr = mkCrateClippyCrane { crate = "cadenza-syntax-sexpr"; };
              clippy-cadenza-syntax-toml = mkCrateClippyCrane { crate = "cadenza-syntax-toml"; };
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
              # rcdzc-cli (v-cdz-crate-split 2026-08-30): the clap CLI layer + `cdz-compile` bin over the
              # rcdzc LIBRARY. Its own targets (lib + bin + one clap unit test) reference no external dirs —
              # rcdzc enters only as a compiled lib DEP (not its tests), so no spec/semantics extraSrc needed.
              clippy-rcdzc-cli = mkCrateClippyCrane { crate = "rcdzc-cli"; };
              clippy-xtask = mkCrateClippyCrane { crate = "xtask"; extraSrc = [ ./spec/semantics ./implementation/compiler-ml ]; extraInputs = [ pkgs.git ]; };
              clippy-xtask-mandates = mkCrateClippyCrane { crate = "xtask-mandates"; };
              clippy-xtask-support = mkCrateClippyCrane { crate = "xtask-support"; };
              clippy-xtask-roundtrip = mkCrateClippyCrane { crate = "xtask-roundtrip"; };
              clippy-xtask-lint-emoji = mkCrateClippyCrane { crate = "xtask-lint-emoji"; };
              clippy-xtask-canonicalize-baselines = mkCrateClippyCrane { crate = "xtask-canonicalize-baselines"; };
              clippy-xtask-fmt = mkCrateClippyCrane { crate = "xtask-fmt"; };
              clippy-xtask-codegen-contracts = mkCrateClippyCrane { crate = "xtask-codegen-contracts"; };
              clippy-xtask-codegen-wasm-abi = mkCrateClippyCrane { crate = "xtask-codegen-wasm-abi"; };
              clippy-xtask-codegen-declines = mkCrateClippyCrane { crate = "xtask-codegen-declines"; };
              clippy-xtask-codegen-guide = mkCrateClippyCrane { crate = "xtask-codegen-guide"; };
              clippy-xtask-prune-baselines = mkCrateClippyCrane { crate = "xtask-prune-baselines"; };
              clippy-xtask-save-baseline = mkCrateClippyCrane { crate = "xtask-save-baseline"; };
              clippy-xtask-merge-baseline = mkCrateClippyCrane { crate = "xtask-merge-baseline"; };
              clippy-xtask-bench = mkCrateClippyCrane { crate = "xtask-bench"; };
              clippy-xtask-install-lsp = mkCrateClippyCrane { crate = "xtask-install-lsp"; };
              clippy-xtask-duvet-check = mkCrateClippyCrane { crate = "xtask-duvet-check"; };
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
              # cadenza-compile-abi: runs its unit tests (7). Leaf, zero deps. REQUIRED by testCrateCoverageAssert.
              test-cadenza-compile-abi = mkCrateTestCrane { crate = "cadenza-compile-abi"; };
              test-cadenza-syntax = mkCrateTestCrane { crate = "cadenza-syntax"; extraSrc = [ ./spec/semantics ]; };
              # cadenza-syntax split (#5076/#5082): leaf surface crates, no tests/ dir, no spec/semantics dep.
              test-cadenza-syntax-cedar = mkCrateTestCrane { crate = "cadenza-syntax-cedar"; };
              test-cadenza-syntax-core = mkCrateTestCrane { crate = "cadenza-syntax-core"; };
              test-cadenza-syntax-json = mkCrateTestCrane { crate = "cadenza-syntax-json"; };
              test-cadenza-syntax-sexpr = mkCrateTestCrane { crate = "cadenza-syntax-sexpr"; };
              test-cadenza-syntax-toml = mkCrateTestCrane { crate = "cadenza-syntax-toml"; };
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
              # wasm-abi-table: runs its baked-in `#[cfg(test)] mod oracle` (155 asserts vs the wasm-encoder
              # spec — the operator's inverted guarantee). Leaf like cdz-wasm-opt-gap: test-crane only, no
              # clippy-crane. REQUIRED by testCrateCoverageAssert now that it is a rootWorkspaceCrates member.
              test-wasm-abi-table = mkCrateTestCrane { crate = "wasm-abi-table"; };
              test-cdz-world-artifact = mkCrateTestCrane { crate = "cdz-world-artifact"; };
              test-rcdzc = mkCrateTestCrane {
                crate = "rcdzc";
                extraSrc = [ ./spec/semantics ./implementation/compiler-ml ./implementation/seed/crates/cdz-runtime/src/bigint.rs ];
              };
              # rcdzc-cli (v-cdz-crate-split 2026-08-30): runs its clap unit test (overflow-flag parsing).
              # REQUIRED by testCrateCoverageAssert now that rcdzc-cli is a workspace member. No extraSrc —
              # rcdzc is a compiled lib dep here, not its own test target.
              test-rcdzc-cli = mkCrateTestCrane { crate = "rcdzc-cli"; };
              test-xtask = mkCrateTestCrane { crate = "xtask"; extraSrc = [ ./spec/semantics ./implementation/compiler-ml ]; extraInputs = [ pkgs.git ]; };
              test-xtask-mandates = mkCrateTestCrane { crate = "xtask-mandates"; };
              test-xtask-support = mkCrateTestCrane { crate = "xtask-support"; };
              test-xtask-roundtrip = mkCrateTestCrane { crate = "xtask-roundtrip"; };
              test-xtask-lint-emoji = mkCrateTestCrane { crate = "xtask-lint-emoji"; };
              test-xtask-canonicalize-baselines = mkCrateTestCrane { crate = "xtask-canonicalize-baselines"; };
              test-xtask-fmt = mkCrateTestCrane { crate = "xtask-fmt"; };
              test-xtask-codegen-contracts = mkCrateTestCrane { crate = "xtask-codegen-contracts"; };
              test-xtask-codegen-wasm-abi = mkCrateTestCrane { crate = "xtask-codegen-wasm-abi"; };
              test-xtask-codegen-declines = mkCrateTestCrane { crate = "xtask-codegen-declines"; };
              test-xtask-codegen-guide = mkCrateTestCrane { crate = "xtask-codegen-guide"; };
              test-xtask-prune-baselines = mkCrateTestCrane { crate = "xtask-prune-baselines"; };
              test-xtask-save-baseline = mkCrateTestCrane { crate = "xtask-save-baseline"; };
              test-xtask-merge-baseline = mkCrateTestCrane { crate = "xtask-merge-baseline"; };
              # xtask-bench: runs its 4 pure unit tests (tolerance, parse_alloc_lines, baseline round-trip, diff
              # classification). Leaf like the other xtask-* bins. REQUIRED by testCrateCoverageAssert.
              test-xtask-bench = mkCrateTestCrane { crate = "xtask-bench"; };
              test-xtask-install-lsp = mkCrateTestCrane { crate = "xtask-install-lsp"; };
              test-xtask-duvet-check = mkCrateTestCrane { crate = "xtask-duvet-check"; };
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
                inherit (perCrateClippyCrane) clippy-rcdzc clippy-rcdzc-cli clippy-cdz-num clippy-cdz-calc clippy-cadenza-syntax clippy-cdz-platform
                  clippy-cdz-component-rewrite clippy-cdz-contract
                  clippy-cadenza-syntax-cedar clippy-cadenza-syntax-core clippy-cadenza-syntax-json
                  clippy-cadenza-syntax-sexpr clippy-cadenza-syntax-toml clippy-cadenza-compile-abi;
              } ''
              echo "ok: clippy shard A — rcdzc + rcdzc-cli + cdz-num + cdz-calc + cadenza-syntax(+core/cedar/json/sexpr/toml) + cadenza-compile-abi + cdz-platform + cdz-component-rewrite + cdz-contract" > $out
            '';
            clippyShardB = pkgs.runCommand "cargo-clippy-shard-b"
              {
                inherit crateCdzCheck;
                inherit (perCrateClippyCrane)
                  clippy-cdz-run clippy-xtask clippy-xtask-mandates clippy-xtask-support clippy-xtask-roundtrip clippy-xtask-lint-emoji clippy-xtask-canonicalize-baselines clippy-xtask-fmt clippy-xtask-codegen-contracts clippy-xtask-codegen-wasm-abi clippy-xtask-codegen-declines clippy-xtask-codegen-guide clippy-xtask-prune-baselines clippy-xtask-save-baseline clippy-xtask-merge-baseline clippy-xtask-bench clippy-xtask-install-lsp clippy-xtask-duvet-check clippy-cadenza-ast clippy-cdz-corpus clippy-cdz-rt clippy-cdz-rust-render;
              } ''
              echo "ok: clippy shard B — cdz (workspace) + cdz-run + xtask + xtask-mandates + xtask-lint-emoji + xtask-canonicalize-baselines + xtask-fmt + xtask-codegen-contracts + xtask-codegen-wasm-abi + xtask-prune-baselines + xtask-bench + cadenza-ast + cdz-corpus + cdz-rt + cdz-rust-render" > $out
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
              # STANDALONE crate (v-xtask-decompose): the round-trip check now runs the `xtask-roundtrip`
              # bin (deps only xtask-support, NOT the xtask monolith). It needs the cdz + cdz-corpus tool
              # binaries, so build those first and hand them to the bin via CDZ_SEED_BIN_DIR (the same env
              # the nix app uses) — the bin then spawns them for the surface conversions. cwd is the
              # seedRoundtripSrc root (carries spec/semantics), so the bin's repo-root cwd fallback finds
              # the corpus. Same tool-build cost as before (the old `xtask roundtrip` cargo-built them too).
              cargoCmd = "cargo build --locked --profile release -p cdz -p cdz-corpus && CDZ_SEED_BIN_DIR=\"$PWD/target/release\" cargo run --locked --profile release -p xtask-roundtrip";
              # SCOPED src (v-nix caching): only the cdz+cdz-corpus+xtask-roundtrip dep-closure has full src;
              # the rest are Cargo.toml-only + stubbed (stubClosure) so cargo loads the workspace. A non-closure
              # crate edit (e.g. cdz-platform) no longer rotates this check. See seedRoundtripSrcScoped's note.
              src = seedRoundtripSrcScoped;
              stubClosure = roundtripClosure;
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
            # cdz-fmt-check (v-code-cleanliness seq-282 #6321; v-nix nix-gate 2026-08-30): the AUTHORITATIVE
            # fleet-wide merge gate for `cdz fmt --check` on the 6 canonical domain src dirs (the local
            # companion is `cargo xtask check`'s cdz-fmt-check step). `cdz fmt` is NOT feature-gated (main.rs
            # `Cmd::Fmt => run_fmt` delegates to cadenza-syntax) — pure front-end, NO store/runtime — so it
            # runs on the CACHED seedCompiler bin over a fileset of ONLY the 6 dirs (cheap + build-hold-safe,
            # no cargo rebuild). SCOPE = these 6 .cdz src dirs ONLY (raw recursion over manifest-less dirs);
            # widening tracks v-code-cleanliness's local gate in lockstep. HISTORY: seq-282 #6818/#6819 briefly
            # WIDENED this to include spec/semantics/*.sexp — but v-parser-corpus inc-6 comment-round-trip churn
            # touches printer.rs nearly every batch (#6863/#6868/#6874), so a merge-required .sexp gate flapped
            # red + forced a re-fmt treadmill. Concierge APPROVED (seq-282 option C, 2026-08-31): the .sexp
            # portion goes ADVISORY — DROPPED from the merge-required fail-set (v-code-cleanliness landed the
            # local companion #6885; this nix gate mirrors it). RE-PROMOTE (v-code-cleanliness will ping): after
            # the inc-6 comment-round-trip series COMPLETES + v-syntax declares the comment/doc printer STABLE,
            # one final `cdz fmt spec/semantics` + re-add spec/semantics here in lockstep. cdz-platform stays OUT
            # (v-platform zone). Exits 0 today (the 6 .cdz src dirs canonical via v-syntax fmt-all #6317/#6319).
            cdzFmtCheckSrc = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./implementation/compiler-ml/src
                ./implementation/cad/src
                ./implementation/music/src
                ./implementation/des/src
                ./implementation/iterators/src
                ./implementation/choreography/src
              ];
            };
            cdzFmtCheck = pkgs.runCommand "cdz-fmt-check" { } ''
              export HOME="$TMPDIR/home"; mkdir -p "$HOME"
              cd ${cdzFmtCheckSrc}
              ${seedCompiler}/bin/cdz fmt --check \
                implementation/compiler-ml/src implementation/cad/src implementation/music/src \
                implementation/des/src implementation/iterators/src implementation/choreography/src
              echo "ok: cdz-fmt-check (6 canonical domain .cdz src dirs — cdz fmt --check clean)" > "$out"
            '';
            # decline-professionalism (v-fleet-tooling gate-wiring 2026-08-31; scan by v-corpus-harness #6791,
            # DEFERRAL_LEXICON owned by v-deferral-declines seq-280): `xtask-mandates declines` — a static
            # source scan of rcdzc/src rust string-literals ensuring NO deferral-trash wording ("yet"/"for
            # now"/"currently"/…) leaks into user-facing decline() messages. FOLDED into the localGate fail-set
            # below → teeth under `gh pr merge --admin` (a GHA required-status can't gate self-merge). RATCHET-
            # AT-ZERO, NO GRANDFATHER: turned on GREEN — v-deferral + v-corpus-harness both verified clean/exit-0
            # on current main (all residue landed: cadenza #6743, link.rs #6751, lower/* #6767, compute+resolve
            # +twin #6775, rust/wasm/emit reworks) — so it starts green with no allowlist. A future legit
            # lexicon-word use is handled by v-deferral tuning the lexicon const, not a gate exception. Same
            # cheap native source-scan shape as mandateLintCheck (reuses the xtask-mandates crate build); the
            # `declines` subcommand runs ONLY this scan, leaving the default run (mandateLintCheck) UNCHANGED.
            declineProfessionalismCheck = cargoWorkspaceCheck {
              name = "cargo-xtask-declines";
              cargoCmd = "cargo run --locked --package xtask-mandates --profile release -- declines";
            };
            mandateLintCheck = cargoWorkspaceCheck {
              name = "cargo-xtask-lint-mandates";
              # STANDALONE crate (v-xtask-decompose): builds ONLY `xtask-mandates` (+ its sole dep syn), NOT
              # the xtask monolith's ~15-dep closure — so a mandate-lint edit no longer recompiles xtask, and
              # the `xtask → xtask-mandates` dep is severed (operator 2026-08-28: cache each subcrate
              # independently). The standalone bin resolves the scan root from cwd (the derivation's seedSrc
              # working dir), so it walks implementation/**/*.rs exactly as before — behavior unchanged.
              # src = seedSrc (default) is kept: it is the SCAN CORPUS the lint reads at runtime (+ carries
              # the xtask-mandates crate src, since seedSrc includes the whole ./xtask).
              cargoCmd = "cargo run --locked --package xtask-mandates --profile release";
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
                inherit clippyShardA clippyShardB codegenCheck gateCheck gateCheckRust
                  # guideExamplesCheck DROPPED from the fail-set → ADVISORY (still exposed as
                  # checks.guide-examples). It runs the guide's serial node check:* battery which COMPILES every
                  # example/preload via the browser compiler-wasm, and that OOBs "memory access out of bounds"
                  # on rustc/LLVM-22's memory.copy/fill + overlong call_indirect (binaryen 131 preserves them;
                  # only wasm-pack's older binaryen 117 lowers them) — a KNOWN multi-facet compiler-wasm bug
                  # tracked as a follow-up (concierge-approved (C) 2026-08-30; the red was already --admin-bypassed
                  # fleet-wide). Replaced here by guideExamplesShredded — the NATIVE Rust-shred 410-example matrix
                  # (cdz-compile→cdz-run, NOT wasm → no OOB) — the authoritative example-compile coverage, so this
                  # makes localGate honestly-green with ZERO example-coverage loss. (guide test:unit/build stay
                  # advisory via checks.guide-examples; real fix = engine upgrade (Path B, v-guide-infra) or a
                  # binaryen-117 pin (Path A).)
                  guideExamplesShredded
                  # guideManifestDriftAssert FOLDED IN (v-guide-infra 2026-08-31): the committed
                  # guide/examples-manifest.json MUST equal a fresh guideShred manifest. It was exposed as
                  # checks.guide-manifest-drift-assert but NOT gated, so #6898 (removed a MapsSets runnable →
                  # shifted the shred enumeration) landed a STALE manifest → guideExamplesShredded's per-case
                  # builds then cp-failed on the renumbered dirs, cascading a CONFUSING red fleet-wide (v-guide-
                  # editor #6960 had to hand-regen). Folding the assert in makes a forgotten manifest regen fail
                  # FAST + LOCAL at the authoring PR with the CLEAR "regen: nix build .#guide-shred && cp …"
                  # message (#6961), not a downstream cp cascade. Near-ZERO gate cost: it reuses the SAME
                  # guideShred derivation guideExamplesShredded already builds (just a jq -S diff on top).
                  # Green-confirmed on current main before the fold (committed == fresh, count=412/emitted=405/
                  # deferred=7). Teeth a required-status can't give under self-merge.
                  guideManifestDriftAssert
                  # guideCdzRenderAssert FOLDED IN (v-guide-infra 2026-09-01): every AST-backed inline
                  # (cdz …) span must render in both surfaces from its embedded binary AST. The node
                  # check:cdz-render is advisory-tier (unenforced under the sole-localGate model), so this
                  # NATIVE twin (cdz convert --from binary, no compile → cheap) gates it at merge. Closes the
                  # inline-span render gap v-guide-editor flagged. Green-confirmed before the fold.
                  guideCdzRenderAssert
                  benchCheck runtimeHashParity fmtCheck testCraneAggregate roundtripCheck
                  # cdzFmtCheck FOLDED IN (v-code-cleanliness seq-282, v-nix 2026-08-30): the AUTHORITATIVE
                  # fleet-wide `cdz fmt --check` gate on the 6 canonical domain src dirs. Cheap front-end
                  # (cached seedCompiler bin, no store/runtime), green-confirmed standalone before the fold.
                  cdzFmtCheck
                  mandateLintCheck cdzRunDependentsAssert standaloneWasmWorkspaceAssert
                  wasmtimeSingleHolderAssert compilerPureLibraryAssert
                  # cdz-wasm NATIVE tests (host, OOB-free) — GATES the browser compiler's sidecar consumers
                  # so a future binary-AST wire flip can't silently re-break them (the #6324/#6342 hole).
                  cdzWasmNativeCheck
                  # corpus-hygiene lints FOLDED IN (v-fleet-tooling 2026-08-30, v-corpus-harness green +
                  # concierge exempt-first-then-fold): corpusNativizeCheck (M3 input #ctor form; #6025 escaped
                  # the ADVISORY checks.yml job because --admin bypasses required GHA) + corpusVanishedCheck
                  # (baseline title-drift; existed as checks.corpus-vanished but was NOT gated). Both cheap
                  # (cdzCorpus + a sexp scan) + green-confirmed on the corpus before the fold, so no gate-time
                  # regression + no false red. This is the teeth a required-status can't give under self-merge.
                  corpusNativizeCheck corpusVanishedCheck
                  # decline-professionalism FOLDED IN (v-fleet-tooling 2026-08-31, scan #6791, lexicon
                  # v-deferral-declines): no seq-280 deferral-trash wording in decline() messages. Ratchet-at-
                  # ZERO — v-deferral + v-corpus-harness both verified clean/exit-0 on current main before this
                  # fold (no grandfather), and it's a cheap rcdzc/src source-scan reusing the xtask-mandates
                  # build, so no false red + ~no gate-time add. Teeth under self-merge like the corpus lints.
                  declineProfessionalismCheck
                  # capability-error FOLDED IN (v-fleet-tooling 2026-08-31, scan v-corpus-harness #6924): no
                  # corpus case pins a capability-limit code (CDZ0900) as an (error …) — the impl-independent-
                  # spec guard. Starts GREEN (v-corpus-harness confirmed 0 hits, no residue → folds immediately,
                  # no fix-then-fold wait); cheap cdzCorpus static parse, same shape as corpusNativizeCheck.
                  capabilityErrorCheck;
                # gateCheckRust folded into the fail-set (v-nix+v-ft 2026-08-10): closes the RUST-backend gate
                # hole — gateCheck is wasm-only, so a rust-only emit divergence (v-effects E0425 mutual-rec)
                # reached trunk green. Narrow `--case mutual` subset (rustc-per-case → full 6686 is prohibitive
                # per-MR); nightly runs the full rust gate. See gateCheckRust's def note.
                # cad-test-compiler-ml REMOVED from the forced fail-set (OPERATOR 2026-08-30): compiler-ml is not
                # changing actively, nothing depends on it, and it was only an effective rust-compiler STRESS
                # test — so stop forcing everyone to run it (the hung `cdz test --warm-only` compiler-ml compiles
                # were a top fleet-CPU starvation source). It STAYS available as an OPT-IN / advisory check
                # (`checks.<sys>.cad-test-compiler-ml`, still built by `cdzCadProjectTests`) — just not a blocking
                # gate. (Was folded in 2026-08-10 as a Core-shape spine guard; a Core-shape edit still rotates
                # seedCompiler + the corpus/cad spine catches gross Core breaks, so dropping the forced spine
                # here is acceptable per the operator.) checks.yml required-set drop = coordinated w/ v-gha-green.
              } ''
              echo "ok: local-gate — 9 merge-required contexts (ruleset-10 minus test-macos) + mandate-lint (compiler-ml now OPT-IN, not forced — operator 2026-08-30), green on aarch64-nix" > $out
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
            cdz-wasm-abi-match = cdzWasmAbiMatch;
            cdz-declines-match = cdzDeclinesMatch;
            # `nix build .#checks.<sys>.decline-professionalism` — the seq-280 decline-message professionalism
            # scan (no deferral-trash wording); also folded into the localGate fail-set (teeth under self-merge).
            decline-professionalism = declineProfessionalismCheck;
            # wasm-abi-oracle: the operator-required derived test — every wasm-abi.sexp byte matches the
            # wasm-encoder oracle (catches a sexpr transcription typo now that the sexp is the source of truth).
            # Standalone (like cdz-wasm-abi-match); runs under `nix flake check`.
            wasm-abi-oracle = wasmAbiOracle;
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
            # The SYNTAX corpus per-case gate (spec/syntax/, inc-3c): one classify derivation per case
            # dir → verdicts harvested + folded vs .gate-baseline via `gate-syntax --compare`. Advisory
            # (per the hourly-advisory land model); the authoritative correctness gate is still the
            # `test-cadenza-syntax` self-consistency run. See DESIGN-parser-test-corpus.md §4.1.
            syntax-corpus = syntaxCorpus;
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
            # The QUOTE binary-AST round-trip pass (v-quote-corpus, DESIGN-quote-corpus-roundtrip-pass): for
            # each eligible case, a §2 two-export component whose `encode-quoted()`→`decode-check(bytes)`
            # round-trip is threaded across the caller boundary by `cdz-run --quote-roundtrip` (+ a
            # corrupt-bytes anti-const-fold negative trial). Per-file `quote-corpus-<file>` aggregates spread
            # below. ADVISORY for now (NOT in the required local-gate set) — a first slice reds only on a
            # compiled program whose round-trip breaks; a baseline/Todo-regression gate is a follow-up.
            quote-corpus = quoteCorpusAll;
            # The GLOBAL half of gap #7: a baseline case with no corpus case (silently dropped, its verdict
            # unenforced) — what the per-case `--baseline` regression check cannot see. Backend-independent.
            corpus-vanished = corpusVanishedCheck;
            corpus-nativize = corpusNativizeCheck;
            # `nix build .#checks.<sys>.capability-error` — no corpus case pins CDZ0900 as an (error …);
            # also folded into the localGate fail-set (teeth under self-merge). Scan by v-corpus-harness #6924.
            capability-error = capabilityErrorCheck;
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
            cdz-run-dependents-assert = cdzRunDependentsAssert;
            wasmtime-single-holder-assert = wasmtimeSingleHolderAssert;
            compiler-pure-library-assert = compilerPureLibraryAssert;
            standalone-wasm-workspace-assert = standaloneWasmWorkspaceAssert;
            # cdz = WORKSPACE-SRC (concierge-confirmed 1a), NOT closure/tests-dir-scoped like the other 10.
            # WHY cdz differs: its run_rust_cli tests are WORKSPACE-INTEGRATION — they rustc-compile emitted
            # Rust linking the sibling cdz-num/cdz-rt rlibs "beside the cdz bin", which only a full-workspace
            # build lays out (a bare `-p cdz` does NOT even emit libcdz_num.rlib — cdz uses cdz-num only
            # transitively via rcdzc → E0433). So crateCdzCheck does `cargo build --workspace` FIRST (lays
            # every rlib) THEN `test -p cdz`, from crateCdzCheckSrc (crates+xtask+compiler-ml; git for xtask
            # fleet batch tests). cdz is the TOP crate (reruns on ~every edit anyway → tests-dir granularity is
            # ~nil), so workspace-src costs ~nothing AND is DRIFT-FREE — a `--test`-exclusion list to keep it
            # closure-scoped would silently drop a new cdz test (the coverage regression the parity guard
            # forbids). Do NOT "fix" this back to a split. NOTE (v-nix caching push 2026-08-29): src is
            # crateCdzCheckSrc = seedTestSrc MINUS spec/semantics — that corpus dir is a test-RUNTIME input for
            # cadenza-syntax only (NOT a cdz build/test input), so dropping it stops the highest-frequency fleet
            # change (corpus migrations) from rotating this whole-workspace check. See crateCdzCheckSrc's note.
            crate-cdz = crateCdzCheck;
            # cdz-default-features: exposed for fast-gate (crateChecks "cdz") + explicit `nix build`. NOT a
            # localGate constituent (crateCdzCheck's workspace build already covers default-features at the
            # authoritative gate) — this is the cheap inner-loop catch for the default-only-arm class.
            cdz-default-features = cdzDefaultFeaturesCheck;
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
            cdz-wasm-native = cdzWasmNativeCheck;
            # Full-CI-in-nix increment 6b: the GHA codegen job (cargo xtask codegen --check, ABI staleness).
            codegen-check = codegenCheck;
            # Full-CI-in-nix increment 6c: the GHA gate job (cargo xtask gate --check — THE behavior gate).
            gate-check = gateCheck;
            # ADVISORY (NOT in the localGate fail-set) — the tiered-opt O0..O3 level-equivalence sweep, wired
            # into nightly.yml by v-gha-green. `nix build .#checks.<sys>.opt-sweep`.
            opt-sweep = optSweepCheck;
            # gate-check-verify: SOLO full-corpus grade with a 30-min/case timeout (for v-effects' UAF-critical
            # verify past the 30s gate cap + the fleet batch cap). NOT in localGate — verify affordance only.
            gate-check-verify = gateCheckVerify;
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
            guide-shred-check = guideShredCheck;
            guide-examples-shredded = guideExamplesShredded;
            guide-manifest-drift-assert = guideManifestDriftAssert;
            guide-cdz-render-assert = guideCdzRenderAssert;
          } // guideFileAggs // testShredFileAggs // {
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
            # cdz-fmt-check: cdz fmt --check on the 6 canonical domain src dirs (v-code-cleanliness seq-282).
            # Folded into localGate's FAIL-SET (below) — the AUTHORITATIVE fleet-wide fmt gate.
            cdz-fmt-check = cdzFmtCheck;
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
          # PER-FILE quote round-trip aggregates: `quote-corpus-<file>` (per-case shred(--quote-wrap)→build→
          # `cdz-run --quote-roundtrip`), so one file's quote per-case graph builds/caches in isolation
          # (top-level `quote-corpus` forces them all).
          // quoteCorpusFileAggs
          # PER-FILE wasm-opt-gap aggregates: `wasm-opt-gaps-<file>` for every corpus file, so a slice
          # (01-literals + 10-bytes) builds in isolation while the top-level `wasm-opt-gaps` forces the whole
          # sweep. Per-CASE reports are CA on {emit, binaryen} → shared with `wasm-opt-gaps` + cached.
          // optGapFileAggs;

        # devShell packages include cdzShellWrappers (the hoisted PATH wrapper-scripts, defined in the let
        # above + shared with packages.cdz-shell-wrappers). Cross-shell (agent Bash-tool subshells are zsh),
        # each execs `nix run <worktree>#app` (rebuild-on-edit). CDZ_STORE/CDZ_COMPILE_BIN set by the apps.
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
          #   git / gh      : ESSENTIAL for agents booting directly into this shell (all-nix cutover) —
          #                   git for every worktree op (pull/commit/branch, + the `__cdz_flakeroot`
          #                   alias helper) and gh for the open-own-PR + self-merge land model. Both are
          #                   external/substitutable, so eager. Don't rely on the host PATH leaking into
          #                   `nix develop` — make the one shell self-sufficient (operator hit `git pull:
          #                   tool git not found`, 2026-08-28).
          packages = [
            rustToolchain
            pkgs.wasm-tools
            pkgs.cargo-component
            pkgs.lean4
            pkgs.git
            pkgs.gh
          ] ++ cdzShellWrappers;

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
          # The all-nix entrypoints are PATH WRAPPER-SCRIPTS in `packages` (cdzWrappers) — cross-shell (they
          # reach the agent's ZSH Bash-tool subshells, which bash `export -f` functions never did) and still
          # rebuild-on-edit (each execs `nix run <worktree>#app`). No shell functions here. CDZ_STORE /
          # CDZ_COMPILE_BIN are injected by the apps themselves, so boot stays LAZY (no local build eager).
          shellHook = ''
            export NIX_REMOTE=daemon
            export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-8}"
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
              # cdz: crate-cdz (workspace test+clippy) PLUS cdz-default-features (the cheap default-only-arm
              # catch — the per-crate crane checks + seedCompiler build --no-default-features, so a touched cdz
              # in the inner loop needs this to catch a watch/lsp/completions/corpus/standalone cfg-arm break).
              if name == "cdz" then [ "crate-cdz" "cdz-default-features" ]
              else [ "test-${name}" "clippy-${name}" ];
            # bash `case` arms mapping a crate DIR PREFIX → its space-joined check attrs (for git-diff detect).
            # MOST-SPECIFIC FIRST: sort crates by dir-path LENGTH descending so a crate NESTED under
            # another (e.g. `xtask/crates/xtask-mandates` under `xtask`) emits its `…/*)` arm BEFORE the
            # parent's broad `xtask/*)`. bash `case` takes the FIRST matching pattern, so without this the
            # broad arm shadows the specific one → shellcheck SC2221/SC2222 fails the cdz-fast-gate build
            # (breaking `nix run .#fast-gate` + `cargo xtask dev-gate` fleet-wide) AND a real mis-grade: an
            # edit under the sub-crate would run the PARENT crate's checks. Length-desc guarantees the
            # prefix invariant (a nested path is strictly longer than the parent it extends). (#5056 fallout)
            dirCaseArms = pkgs.lib.concatStringsSep "\n" (map
              (c: ''            ${rootWorkspaceCrates.${c}}/*) echo "${pkgs.lib.concatStringsSep " " (crateChecks c)}" ;;'')
              (pkgs.lib.sort
                (a: b: builtins.stringLength rootWorkspaceCrates.${a} > builtins.stringLength rootWorkspaceCrates.${b})
                rootCrateNames));
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

        # apps.test — per-crate nix TEST (operator NIX-FIRST-for-tests 2026-08-28, sccache SHELVED). The
        # agent-facing surface v-fleet-tooling's `cargo test -p CRATE` → nix redirect targets: SINGLE app
        # (their design call — less surface + auto-handles new crates). Each crate's tests run via the
        # existing per-crate crane check (`test-<crate>` — inherits the shared cargoArtifacts deps layer, so
        # deps compile ONCE fleet-wide and only the top crate + its dependents recompile), and cdz via the
        # combined `crate-cdz` workspace check. No arg → the full per-crate test aggregate (all crates).
        #   nix run .#test -- cdz-num cadenza-syntax   # named crates
        #   nix run .#test                             # everything (test-crane-aggregate)
        # NOT added to cdz-shell-wrappers: a PATH command named `test` would shadow the shell/coreutils
        # `test` builtin — this stays the explicit `nix run .#test` surface the cargo-shim maps onto.
        apps.test =
          let
            # crate name → its TEST check. cdz is the combined crate-cdz (its clippy+test run in one
            # workspace-src check); every other root crate has a per-crate crane `test-<c>`. Auto-covers a
            # new crate (rootCrateNames is the workspace-member map; testCrateCoverageAssert keeps it honest).
            testCheckFor = name: if name == "cdz" then "crate-cdz" else "test-${name}";
            nameCaseArms = pkgs.lib.concatStringsSep "\n" (map
              (c: ''            ${c}) echo "${testCheckFor c}" ;;'')
              rootCrateNames);
            testApp = pkgs.writeShellApplication {
              name = "cdz-test";
              runtimeInputs = [ pkgs.nix pkgs.coreutils pkgs.git ];
              text = ''
                name_check() {
                  case "$1" in
                ${nameCaseArms}
                    *) echo "" ;;
                  esac
                }
                root="$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")"
                if [ "$#" -eq 0 ]; then
                  echo "cdz test: no crate arg — running the FULL per-crate test aggregate (all workspace crates; deps cached, top-crate recompile)"
                  exec nix build --print-build-logs "$root#checks.${system}.test-crane-aggregate"
                fi
                checks=""
                for c in "$@"; do
                  got="$(name_check "$c")"
                  if [ -z "$got" ]; then echo "cdz test: '$c' is not a gated root crate — skipping" >&2; else checks="$checks $got"; fi
                done
                checks="$(echo "$checks" | tr ' ' '\n' | sort -u | grep -v '^$' | tr '\n' ' ')"
                if [ -z "$checks" ]; then echo "cdz test: no valid root crate in args — nothing to run" >&2; exit 1; fi
                attrs=""; for c in $checks; do attrs="$attrs $root#checks.${system}.$c"; done
                echo "cdz test: building (deps cached, top-crate recompile):$checks"
                # shellcheck disable=SC2086
                if nix build $attrs --print-build-logs; then
                  echo "cdz test: GREEN — the requested crate(s) pass their test suite."
                else
                  echo "cdz test: RED — a crate's tests failed above. Fix + re-run." >&2
                  exit 1
                fi
              '';
            };
          in
          {
            type = "app";
            program = "${testApp}/bin/cdz-test";
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

        # apps.roundtrip — the FIRST per-subcommand xtask nix app (v-xtask-decompose proof-of-pattern,
        # operator all-nix mandate 2026-08-28). Decomposes `cargo xtask roundtrip` into a nix-native app an
        # agent runs as `nix run .#roundtrip -- [files…]` — it auto-builds the warm-cached `xtaskBin` (no bare
        # cargo, no per-worktree cold rebuild) and forwards args. Roundtrip is the cleanest first subcommand:
        # PURE (reads spec/semantics + cadenza-syntax only — no runtime store, no cdz-run, no component build),
        # so it isolates the wrapper+relocatability pattern from the heavier store-dependent subcommands
        # (gate/build) that come next. RELOCATABILITY: the nix-built binary baked its build-sandbox path into
        # `CARGO_MANIFEST_DIR`, so it can't self-locate the repo at runtime; the wrapper resolves the invoking
        # worktree via `git rev-parse --show-toplevel` and exports `CDZ_REPO_ROOT`, which `Paths::resolve`
        # honors (falling back to CARGO_MANIFEST_DIR when unset — the unchanged `cargo xtask` path). Every
        # subsequent per-subcommand app reuses this wrapper shape.
        apps.roundtrip =
          let
            # The warm nix-built pipeline tools roundtrip needs (cdz for the surface conversions, cdz-corpus
            # for corpus normalization) as ONE bin dir, so `build_tools` takes the CDZ_SEED_BIN_DIR override
            # and SKIPS its internal `cargo build -p cdz -p cdz-corpus …` — no per-worktree cold toolchain
            # rebuild at `nix run` time (v-xtask-decompose: the "don't rebuild the world" half). seedCompiler
            # carries cdz + cdz-run; cdzCorpus carries cdz-corpus; symlinkJoin merges their bin/ dirs.
            seedTools = pkgs.symlinkJoin {
              name = "cdz-seed-tools";
              paths = [ seedCompiler cdzCorpus ];
            };
            wrapper = pkgs.writeShellApplication {
              name = "cdz-roundtrip";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                export CDZ_SEED_BIN_DIR="${seedTools}/bin"
                exec ${xtaskRoundtripBin}/bin/xtask-roundtrip "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-roundtrip";
          };

        # apps.fmt — the Cadenza formatter as a nix-native app backed by the STANDALONE `xtaskFmtBin`
        # (v-xtask-decompose). `nix run .#fmt -- [--to <surface>] [--check] <file>…`. Builds ONLY xtask-fmt
        # (+ xtask-support), NOT the xtask monolith; the `Cmd::Fmt` arm is removed so `cargo xtask fmt`
        # forwards here via the cargo→nix redirect. Needs only `cdz` (surface convert), so CDZ_SEED_BIN_DIR
        # points at seedCompiler's bin/ (no cdz-corpus, unlike roundtrip) — cargo-free. Sets CDZ_REPO_ROOT
        # only for the dev-fallback bin dir.
        apps.fmt =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-fmt";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                export CDZ_SEED_BIN_DIR="${seedCompiler}/bin"
                exec ${xtaskFmtBin}/bin/xtask-fmt "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-fmt";
          };

        # apps.world-artifact — the WIT-world artifact utility as a nix-native app backed by the crane-built
        # `cdzWorldArtifactBin` (v-xtask-decompose). `nix run .#world-artifact -- [world]`. Replaces the old
        # `cargo xtask world-artifact` (which cargo-BUILT cdz-world-artifact then shelled out — a bare cargo
        # call the all-nix mandate forbids); the `Cmd::WorldArtifact` arm is removed so `cargo xtask
        # world-artifact` forwards here. The wrapper supplies the same defaults xtask did (wit =
        # cdz-platform/wit/world.wit, out = target/wit-worlds) and passes any trailing arg through as the
        # optional single-world filter (matches the CLI's positional `<wit> <out> [world]`).
        apps.world-artifact =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-world-artifact-run";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                exec ${cdzWorldArtifactBin}/bin/cdz-world-artifact \
                  "$root/implementation/seed/crates/cdz-platform/wit/world.wit" \
                  "$root/target/wit-worlds" "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-world-artifact-run";
          };

        # apps.lint-mandates — the mandate-lint as a nix-native app backed by the STANDALONE
        # `xtaskMandatesBin` (v-xtask-decompose). `nix run .#lint-mandates`. This is what makes the crate
        # split pay off: the app builds ONLY xtask-mandates (+ syn), NOT the xtask monolith — and with the
        # `xtask → xtask-mandates` dep now SEVERED (the Cmd::LintMandates arm removed), the two cache fully
        # independently (operator 2026-08-28). v-fleet-tooling's cargo→nix redirect maps `cargo xtask
        # lint-mandates` here. Sets CDZ_REPO_ROOT so the relocated bin finds the invoking worktree.
        apps.lint-mandates =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-lint-mandates";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskMandatesBin}/bin/xtask-mandates "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-lint-mandates";
          };

        # apps.lint-declines — the decline-message PROFESSIONALISM scan as a nix-native app:
        # `nix run .#lint-declines` (v-fleet-tooling gate-wiring 2026-08-31; scan #6791, lexicon
        # v-deferral-declines seq-280). Same STANDALONE xtaskMandatesBin as apps.lint-mandates, with the
        # `declines` subcommand baked in — a clean alias for `nix run .#lint-mandates -- declines`, giving
        # agents a one-liner to self-check their decline() messages before landing (the same scan folded into
        # localGate as `checks.<sys>.decline-professionalism`). Sets CDZ_REPO_ROOT to the invoking worktree.
        apps.lint-declines =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-lint-declines";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskMandatesBin}/bin/xtask-mandates declines "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-lint-declines";
          };

        # apps.lint-emoji — the emoji-ban source lint as a nix-native app backed by the STANDALONE
        # `xtaskLintEmojiBin` (v-xtask-decompose). `nix run .#lint-emoji`. Builds ONLY xtask-lint-emoji (+
        # xtask-support), NOT the xtask monolith — and with the `Cmd::LintEmoji` arm removed, `cargo xtask
        # lint-emoji` forwards here via v-fleet-tooling's cargo→nix redirect. Sets CDZ_REPO_ROOT so the
        # relocated bin lints the invoking worktree. Mirrors apps.lint-mandates.
        apps.lint-emoji =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-lint-emoji";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskLintEmojiBin}/bin/xtask-lint-emoji "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-lint-emoji";
          };

        # apps.canonicalize-baselines — the `.gate-baseline*` canonicalizer as a nix-native app backed by
        # the STANDALONE `xtaskCanonicalizeBaselinesBin` (v-xtask-decompose). `nix run .#canonicalize-baselines`.
        # Builds ONLY xtask-canonicalize-baselines (+ xtask-support), NOT the xtask monolith — and with the
        # `Cmd::CanonicalizeBaselines` arm removed, `cargo xtask canonicalize-baselines` forwards here via
        # v-fleet-tooling's cargo→nix redirect. Sets CDZ_REPO_ROOT so the relocated bin sweeps the invoking
        # worktree's baselines. Mirrors apps.lint-emoji.
        apps.canonicalize-baselines =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-canonicalize-baselines";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskCanonicalizeBaselinesBin}/bin/xtask-canonicalize-baselines "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-canonicalize-baselines";
          };

        # apps.prune-baselines — the `.gate-baseline*` unreferenced-entry pruner as a nix-native app backed
        # by the STANDALONE `xtaskPruneBaselinesBin` (v-xtask-decompose). `nix run .#prune-baselines [-- --check]`.
        # Builds ONLY xtask-prune-baselines (+ xtask-support), NOT the xtask monolith; the `Cmd::PruneBaselines`
        # arm is removed so `cargo xtask prune-baselines` forwards here. Needs the corpus title set → point
        # CDZ_SEED_BIN_DIR at the nix-built cdz-corpus (`cdzCorpus`), cargo-free (no `cargo build -p cdz-corpus`).
        apps.prune-baselines =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-prune-baselines";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                export CDZ_SEED_BIN_DIR="${cdzCorpus}/bin"
                exec ${xtaskPruneBaselinesBin}/bin/xtask-prune-baselines "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-prune-baselines";
          };

        # apps.bench — the runtime allocation benchmark as a nix-native app backed by the STANDALONE
        # `xtaskBenchBin` (v-xtask-decompose). `nix run .#bench [-- --save]`. Builds ONLY xtask-bench (a
        # std-only leaf), NOT the xtask monolith; the `Cmd::Bench` arm is removed so `cargo xtask bench`
        # forwards here via the cargo→nix redirect. The bin shells `cargo test` in cdz-runtime for the
        # measurement, so cargo/rustc come from the invoking dev shell (same as the old `cargo xtask bench`);
        # the wrapper only resolves + exports CDZ_REPO_ROOT so the bin diffs the invoking worktree's baseline.
        # apps.install-lsp — the LSP-installer as a nix-native app backed by the STANDALONE xtaskInstallLspBin
        # (v-xtask-decompose). `nix run .#install-lsp [-- --uninstall]`. Builds ONLY xtask-install-lsp (a
        # std+xshell leaf), NOT the xtask monolith; the `Cmd::InstallLsp` arm is removed so `cargo xtask
        # install-lsp` forwards here. Sets CDZ_REPO_ROOT so the relocated bin finds the invoking worktree
        # (it symlinks integrations/vscode + builds the cdz LSP server from there); args pass through for --uninstall.
        apps.install-lsp =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-install-lsp";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskInstallLspBin}/bin/xtask-install-lsp "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-install-lsp";
          };

        # apps.duvet-check — the duvet citation-floor check as a nix-native app backed by xtaskDuvetCheckBin
        # (v-xtask-decompose). `nix run .#duvet-check [-- --save]`. Builds ONLY xtask-duvet-check (serde_json+std
        # leaf); the `Cmd::DuvetCheck` arm is removed so `cargo xtask duvet-check` forwards here. CDZ_REPO_ROOT
        # set (it reads duvet report + the committed floor from the worktree); args pass through for --save.
        apps.duvet-check =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-duvet-check";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskDuvetCheckBin}/bin/xtask-duvet-check "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-duvet-check";
          };

        # apps.save-baseline — regenerate spec/semantics/.gate-baseline from the nix corpus-verdicts harvest
        # (v-xtask-decompose seq-202 --save gate-delete replacement for `cargo xtask gate --save`). `nix run
        # .#save-baseline` builds `.#corpus-verdicts` (the whole-corpus <tag>\t<description> harvest, via the
        # cached per-case shred→build→verdict graph) then runs xtask-save-baseline to write the baseline via
        # serialize_baseline. Regenerates ALL THREE backends (wasm + rust + rust-async) from their nix verdict
        # harvests — the faithful re-baseline (the native `cargo xtask gate --save` path is unfaithful for
        # rust/rust-async, #6970). v-corpus-harness reviews the resulting baseline diffs (only-gains + a
        # fail-spike guard) before committing.
        apps.save-baseline =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-save-baseline";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                # Build each backend's whole-corpus verdict harvest (uses the ambient nix the caller ran with),
                # then write its .gate-baseline* via the xtask-save-baseline leaf. All 3 = the faithful nix
                # re-baseline; the cached per-case graphs mean an unchanged corpus is a store cache hit.
                # COARSE harvest (v-nix coarsening 2026-09-01): the -coarse variants are ONE derivation per FILE
                # (~35, each compiling+grading its cases internally) instead of the per-CASE __contentAddressed
                # graph (tens of thousands of CA outputs). The old per-case harvest wedged "live-but-sleeping"
                # under a REALISATION (`.doi`) query storm (a query per CA output per substituter; substitute=false
                # was the blunt workaround, but rebuilt the toolchain from source too / timed out at 5.5h). The
                # coarse graph has only ~35 CA shreds → the storm cannot recur, so substitution stays ON (keeps
                # cachix reuse — a fresh re-baseline builds; an unchanged corpus is a cache hit). VERDICT-SAFE:
                # byte-identical to the per-case harvest (v-corpus-harness parity sign-off — wasm 6-file diverse
                # sample + rust/rust-async 11-modules+05-compound-types all byte-clean). The GATE keeps the
                # per-case granularity (fast incremental PR gating); only this HARVEST coarsens.
                harvest="$(nix build "$root#corpus-verdicts-coarse" --no-link --print-out-paths)"
                ${xtaskSaveBaselineBin}/bin/xtask-save-baseline "$harvest" "$root/spec/semantics/.gate-baseline"
                harvest_rust="$(nix build "$root#corpus-verdicts-rust-coarse" --no-link --print-out-paths)"
                ${xtaskSaveBaselineBin}/bin/xtask-save-baseline "$harvest_rust" "$root/spec/semantics/.gate-baseline-rust"
                harvest_rust_async="$(nix build "$root#corpus-verdicts-rust-async-coarse" --no-link --print-out-paths)"
                exec ${xtaskSaveBaselineBin}/bin/xtask-save-baseline "$harvest_rust_async" "$root/spec/semantics/.gate-baseline-rust-async"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-save-baseline";
          };

        apps.bench =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-bench";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskBenchBin}/bin/xtask-bench "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-bench";
          };

        # apps.xtask — the GENERAL xtask entrypoint through nix (v-nix, operator all-nix mandate 2026-08-28:
        # "delegate the `cargo xtask` alias to use nix"). `nix run .#xtask -- <subcommand> [args…]` runs the
        # warm-cached `xtaskBin` (shared ~383MB dep-closure layer) instead of a bare per-worktree cargo build,
        # forwarding ALL subcommands (fleet / gate-local / world-artifact / …). This is the target the planned
        # `cargo`-shim delegates `cargo xtask …` to (a ~/.local/bin `cargo` that shadows cargo, routes the
        # `xtask` subcommand here, and passes everything else to the real cargo — no `.cargo/config.toml` edit,
        # so it does NOT bust the wasmtime/cranelift deps layer). Same relocatability seam as apps.roundtrip:
        # resolve the invoking worktree via `git rev-parse` and export CDZ_REPO_ROOT so the sandbox-baked
        # CARGO_MANIFEST_DIR is overridden (else fleet/gate paths resolve into the nix sandbox). packages.xtask
        # (pname `cdz-xtask`, bin `xtask`) has no mainProgram, so a bare `nix run .#xtask` mis-inferred the
        # program name — this app is the correct run surface.
        apps.xtask =
          let
            wrapper = pkgs.writeShellApplication {
              name = "cdz-xtask-app";
              runtimeInputs = [ pkgs.git ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                export CDZ_REPO_ROOT="$root"
                exec ${xtaskBin}/bin/xtask "$@"
              '';
            };
          in
          {
            type = "app";
            program = "${wrapper}/bin/cdz-xtask-app";
          };

        #
        # WRAPPED (not a bare bin): the nix `seedCompiler` builds `cdz` in DELEGATE mode
        # (`--no-default-features`, v-cdz-delegate #3397) — so `cdz compile` SPAWNS the external
        # `cdz-compile` binary rather than linking rcdzc, and needs `$CDZ_COMPILE_BIN` set (else
        # `cdz: cdz-compile not found`). And `cdz run`/`cdz test` resolve the runtime/NFC/guest components
        # via `$CDZ_STORE`. So each app is a thin wrapper that injects both (respecting a caller override
        # via `:-`), exactly as the flake's corpus checks do (flake.nix ~L665/1555) — making the app
        # SELF-CONTAINED (works outside `nix develop` too). Because `nix run .#cdz` evaluates the CURRENT
        # (dirty) flake, `cdzCompile`/`componentStore` still rebuild-on-edit from the worktree.
        # apps.cdz / apps.cdz-run — the self-contained front-end via the hoisted wrappers (cdzHandWrapper /
        # cdzRunHandWrapper, defined near cdzRun so apps.build materializes the SAME wrapper — no drift). The
        # wrappers export CDZ_COMPILE_BIN/CDZ_STORE/CDZ_RUN_BIN/CDZ_CALC_BIN (caller-override :-) then exec the
        # seed bin, so a bare `nix run .#cdz -- compile prog.cdz` (or the apps.build-materialized target/release/cdz)
        # uses the warm nix compiler + store, never shelling to nix/cargo. Rebuilds-on-edit (dirty-flake eval).
        apps.cdz = {
          type = "app";
          program = "${cdzHandWrapper}/bin/cdz";
        };
        apps.cdz-run = {
          type = "app";
          program = "${cdzRunHandWrapper}/bin/cdz-run";
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
        # apps.build — the ALL-NIX replacement for `cargo xtask build` (operator all-nix mandate 2026-08-29:
        # "no more ad-hoc cargo runs, no more bloated target directories"). `cargo xtask build` recompiles the
        # value-heap runtime + front-end bins FROM SOURCE into the worktree's target/ — the #1 rebuild-the-world
        # hotspot (174 worktrees × multi-GB target/, cross-worktree redundant cdz/rcdzc/dep rebuilds). But every
        # output it produces ALREADY exists as a shared nix derivation: the front-end bins (seedCompiler's
        # cdz+cdz-run, cdzCompile's cdz-compile) and the content-addressed value-heap runtime store
        # (componentStore = the NFC + release + debug-counters heaps, byte-identical to what `xtask build`
        # writes to target/cadenza-store). So this app just MATERIALIZES those shared-store outputs into the
        # worktree's expected paths as SYMLINKS — zero cargo, a target/ of only symlinks (no bloat) — preserving
        # the `./target/release/cdz` + `target/cadenza-store` contract the loops/scripts use. The cargo→nix shim
        # (v-fleet-tooling) redirects `cargo xtask build` here. For a self-contained run, `nix run .#cdz` is the
        # peer (it exports CDZ_STORE/CDZ_COMPILE_BIN); this app is the "give me the warm bins + store" surface.
        apps.build =
          let
            builder = pkgs.writeShellApplication {
              name = "cdz-build";
              runtimeInputs = [ pkgs.git pkgs.coreutils ];
              text = ''
                root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
                mkdir -p "$root/target/release" "$root/target/cadenza-store"
                # Front-end bins from the SHARED /nix/store (never a per-worktree cargo build). cdz + cdz-run are
                # the SELF-CONTAINED WRAPPERS (same ones apps.cdz/apps.cdz-run use) — they export
                # CDZ_COMPILE_BIN/CDZ_STORE/CDZ_RUN_BIN so a hand-run `./target/release/cdz compile prog.cdz`
                # works with the warm nix compiler + store (never shelling to nix/cargo). cdz-compile is the raw
                # bin (compile-only, no store/env needed; it carries the injected runtime hash).
                ln -sfn ${cdzHandWrapper}/bin/cdz "$root/target/release/cdz"
                ln -sfn ${cdzRunHandWrapper}/bin/cdz-run "$root/target/release/cdz-run"
                ln -sfn ${cdzCompile}/bin/cdz-compile "$root/target/release/cdz-compile"
                # The content-addressed value-heap runtime store: symlink each artifact into a REAL (writable)
                # target/cadenza-store dir (not a dir-symlink to the RO store) so reads hit the shared heaps AND
                # cdz can still write new per-program components alongside.
                for f in ${componentStore}/*; do
                  ln -sfn "$f" "$root/target/cadenza-store/$(basename "$f")"
                done
                echo "cdz build (all-nix): linked front-end bins + value-heap store from the shared /nix/store — no cargo, no bloated target/." >&2
                echo "  cdz         -> ${seedCompiler}/bin/cdz" >&2
                echo "  cdz-run     -> ${cdzRun}/bin/cdz-run" >&2
                echo "  cdz-compile -> ${cdzCompile}/bin/cdz-compile" >&2
                echo "  store       -> ${componentStore} (CDZ_STORE=$root/target/cadenza-store)" >&2
              '';
            };
          in
          {
            type = "app";
            program = "${builder}/bin/cdz-build";
          };

        apps.gate =
          let
            gate = pkgs.writeShellApplication {
              name = "cdz-gate";
              runtimeInputs = [ pkgs.nix ];
              text = ''
                echo "cdz gate: nix build .#checks.${system}.local-gate (full battery; CONVENIENCE — the" >&2
                echo "          authoritative merge gate is 'cargo xtask fleet gate-local')…" >&2
                # --keep-going: build+report EVERY sub-check even when a sibling fails, so one run surfaces the
                # FULL failing set — not a drip (breaker/concierge gate-hygiene 2026-08-29: ~18 gate-check
                # failures stayed masked 8 cycles behind faster-failing siblings that aborted the build early).
                # (The AUTHORITATIVE gate-local's nix_gate_argv in fleet.rs is v-ft's lane — recommended there
                # too; this keeps the convenience surface consistent.)
                exec nix build ".#checks.${system}.local-gate" --keep-going --print-build-logs "$@"
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
