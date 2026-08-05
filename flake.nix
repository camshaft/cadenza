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
        crateDirectDeps = name:
          let
            manifest = builtins.fromTOML
              (builtins.readFile (./. + "/${rootWorkspaceCrates.${name}}/Cargo.toml"));
            depsIn = section: manifest.${section} or { };
            edgesIn = section:
              builtins.filter (d: builtins.elem d rootCrateNames)
                (builtins.filter
                  (d: let v = (depsIn section).${d}; in builtins.isAttrs v && (v ? path))
                  (builtins.attrNames (depsIn section)));
          in
          pkgs.lib.unique (pkgs.lib.concatMap edgesIn
            [ "dependencies" "dev-dependencies" "build-dependencies" ]);
        # transitive closure (incl. self) via a fixpoint over crateDirectDeps.
        crateClosure = start:
          let
            step = acc:
              let next = pkgs.lib.unique (acc ++ pkgs.lib.concatMap crateDirectDeps acc);
              in if builtins.length next == builtins.length acc then acc else step next;
          in pkgs.lib.sort (a: b: a < b) (step [ start ]);
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
          # tree without this (same guard mkCrateCheck's buildPhase uses before stubNonClosure).
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
        };
        # a per-crate clippy+test check, src-scoped to C's dep-closure (COMPILE src only) + non-closure
        # manifests + synthetic stubs + ONLY C's tests/ + root manifest/lock/.cargo/toolchain + extraSrc.
        # extraSrc = non-member paths a crate's build/tests read (spec/semantics, compiler-ml, cdz-runtime
        # bigint).
        mkCrateCheck = { crate, extraSrc ? [ ], extraInputs ? [ ] }:
          let closure = crateClosure crate; in
          pkgs.stdenvNoCC.mkDerivation {
            pname = "cargo-crate-${crate}";
            version = "0.0.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions (
                (pkgs.lib.concatMap crateCompileSrc closure)  # closure crates: Cargo.toml + src/ (NO tests/)
                ++ nonClosureManifests closure                # everyone else: Cargo.toml ONLY
                ++ pkgs.lib.optional
                  (builtins.pathExists (./. + "/${rootWorkspaceCrates.${crate}}/tests"))
                  (./. + "/${rootWorkspaceCrates.${crate}}/tests")  # ONLY the under-test crate's tests/
                ++ [ ./Cargo.toml ./Cargo.lock ./.cargo ./rust-toolchain.toml ]
                ++ extraSrc);
            };
            nativeBuildInputs = [ rustToolchain ] ++ extraInputs;
            buildPhase = ''
              runHook preBuild
              ${mkCargoVendorEnv { vendor = seedCargoVendor; }}
              # materialize synthetic empty target stubs for non-closure members (the src fileset omits
              # their real src for isolation; cargo still needs a target to parse the workspace).
              chmod -R u+w .
              ${stubNonClosure closure}
              cargo clippy -p ${crate} --all-targets --locked -- -D warnings
              cargo test -p ${crate} --locked
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              echo "ok: cargo-crate-${crate} (clippy -p + test -p; closure/tests-dir scoped)" > "$out"
              runHook postInstall
            '';
          };
        # crane MR2: per-crate CLIPPY via crane, consuming the shared cargoArtifacts (deps pre-compiled) so
        # only C's first-party src recompiles — NOT the whole dep closure every run (the ~14m→~6-7m win).
        #
        # craneCrateCommon: the SHARED per-crate crane inputs both the clippy + test makers compose with — the
        # SAME per-crate isolation fileset + stub machinery as mkCrateCheck (a crate's check invalidates only on
        # its closure's src). ONE home for these invariants (fileset scoping, chmod+stub preBuild, cargoArtifacts,
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
        # per-crate CLIPPY via crane. cargoClippyExtraArgs mirrors mkCrateCheck's `cargo clippy -p C
        # --all-targets -- -D warnings`; crane's cargoClippy INJECTS --locked (do NOT add it — a 2nd errors
        # "cannot be used multiple times", #2273). Strict pattern (like craneCrateCommon) so a typo'd key is
        # caught HERE at the call-contract, not late/silently inside the helper (github-liaison #2282); @args
        # still forwards the full attrset to craneCrateCommon — no behavior change, just the strict interface back.
        mkCrateClippyCrane = { crate, extraSrc ? [ ], extraInputs ? [ ] }@args:
          craneLib.cargoClippy ((craneCrateCommon args) // {
            pname = "cargo-clippy-${crate}";
            cargoClippyExtraArgs = "-p ${crate} --all-targets -- -D warnings";
          });
        # per-crate TEST via crane, the TEST-half twin of mkCrateClippyCrane.
        # 🪤 --locked: unlike cargoClippy (which INJECTS --locked), crane's cargoTest does NOT — emits `cargo
        # test --release -p C` (verified). So --locked IS added to cargoExtraArgs for reproducibility parity
        # with the old `cargo test -p C --locked`. (Opposite of the clippy case — #2273.) Strict pattern (same
        # as mkCrateClippyCrane) so a typo'd key is caught at the call-contract; @args forwards to craneCrateCommon.
        mkCrateTestCrane = { crate, extraSrc ? [ ], extraInputs ? [ ] }@args:
          craneLib.cargoTest ((craneCrateCommon args) // {
            pname = "cargo-test-${crate}";
            cargoExtraArgs = "-p ${crate} --locked";
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
        mkCadenzaComponent = { name, cdzFile, componentName ? "cadenza:agent-kernel/fold" }:
          pkgs.stdenvNoCC.mkDerivation {
            pname = name;
            version = "0.0.0";
            src = reducerCadenzaTestSrc; # fixture-dir-rooted → the reducer .cdz are at the src top level.
            nativeBuildInputs = [ seedCompiler ];
            buildPhase = ''
              runHook preBuild
              export HOME="$TMPDIR/home"; mkdir -p "$HOME"
              # compile the single reducer .cdz → a wasm component (emitted to component.wasm in the cwd).
              cdz compile ${cdzFile} --target wasm --component-name ${componentName} -o component.wasm
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
          };
        reducerCadenzaB1 = mkCadenzaComponent { name = "reducer-cadenza-b1"; cdzFile = "reducer_b1.cdz"; };
        reducerCadenzaB2 = mkCadenzaComponent { name = "reducer-cadenza-b2"; cdzFile = "reducer_b2.cdz"; };
        reducerCadenzaB3 = mkCadenzaComponent { name = "reducer-cadenza-b3"; cdzFile = "reducer_b3.cdz"; };
        reducerCadenzaGenesis = mkCadenzaComponent { name = "reducer-cadenza-genesis"; cdzFile = "reducer_genesis.cdz"; };

        # Full-CI-in-nix increment 6e: the GHA `cad-tests` job — `cdz test` on the 4 committed
        # in-tree Cadenza PROJECTS (implementation/{cad,compiler-ml,choreography,iterators}). These are
        # pure-Cadenza (Project.cdz + src/*.cdz), NOT the excluded Rust cdz-cad crate — so no cmake/C++,
        # just the S3 testCadenzaProject pattern applied to real project dirs: the nix-built seedCompiler
        # runs each project's @test suite, resolving the value-heap runtime from my componentStore
        # (CDZ_STORE) — skipping the CI job's `xtask build` + native cdz rebuild. Each project is
        # self-contained (`modules = ["src/*.cdz"]`, no cross-dir imports). Advisory-by-omission →
        # unilateral cargo-twin retire once green.
        cdzCadProjectsSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./implementation/cad
            ./implementation/compiler-ml
            ./implementation/choreography
            ./implementation/iterators
          ];
        };
        cdzCadTestsCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-cad-tests";
          version = "0.0.0";
          src = cdzCadProjectsSrc;
          nativeBuildInputs = [ seedCompiler ];
          buildPhase = ''
            runHook preBuild
            set -o pipefail
            export HOME="$TMPDIR/home"; mkdir -p "$HOME"
            export CDZ_STORE="${componentStore}"
            # Run each project's @test suite explicitly (its dir), resolving the runtime from the nix
            # store. A non-zero `cdz test` propagates (pipefail) and fails the build.
            for p in implementation/cad implementation/compiler-ml \
                     implementation/choreography implementation/iterators; do
              echo "== cdz test $p =="
              cdz test "$p" | tee -a "$TMPDIR/cad-tests.out"
            done
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            cp "$TMPDIR/cad-tests.out" "$out"
            runHook postInstall
          '';
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
        cdzAgentHostNativeCheck = pkgs.stdenv.mkDerivation {
          pname = "cdz-agent-host-native";
          version = "0.0.0";
          src = cdzAgentHostSrc;
          nativeBuildInputs = [ rustToolchain pkgs.cmake pkgs.pkg-config ];
          # cmake is here for aws-lc-sys's build script to CALL, not to configure THIS derivation (no
          # CMakeLists.txt) — disable cmake's configure setup-hook so it doesn't hijack configurePhase.
          dontUseCmakeConfigure = true;
          buildPhase = ''
            runHook preBuild
            # cdz-agent-host has a GIT dependency (s2n-quic-dc-metrics) — mkCargoVendorEnv's default
            # (merged = false) sources the vendor's own config.toml, which carries the git source-
            # replacement stanza, so the offline build resolves the git crate from the vendor.
            ${mkCargoVendorEnv { vendor = cdzAgentHostVendor; }}
            cd implementation/seed/crates/cdz-agent-host
            # Feed the pre-built, pre-validated guest components (my derivations) so the env-gated cedar
            # authz + ComponentSessionFactory e2es RUN instead of skipping.
            export CEDAR_POLICY_COMPONENT="${cedarGuest}"
            export CDZ_REDUCER_COMPONENT="${reducerGuest}"
            cargo test --locked
            cargo clippy --all-targets --locked -- -D warnings
            cargo fmt --check
            cargo test --locked --features admin
            cargo clippy --all-targets --locked --features admin -- -D warnings
            cargo clippy --all-targets --locked --features live-net -- -D warnings
            cargo clippy --all-targets --locked --features admin,live-net -- -D warnings
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-agent-host native (test + clippy + fmt + feature matrix)" > "$out"
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
          # `cdz-run` resolves the runtime's NFC dependency (FINDING#23) by reading `runtime.toml` from the
          # store (the `nfc = "<hash>"` line → `<store>/<hash>.wasm`), and the runtime/debug hashes from
          # it too — WITHOUT this manifest every heap case that composes the runtime fails to resolve NFC.
          # `xtask build` writes exactly this file (main.rs:466); mirror its format so a program run against
          # THIS nix store composes identically to one run against target/cadenza-store.
          rt=$(${pkgs.coreutils}/bin/sha256sum ${runtime}      | ${pkgs.coreutils}/bin/cut -d' ' -f1)
          dbg=$(${pkgs.coreutils}/bin/sha256sum ${runtimeDebug} | ${pkgs.coreutils}/bin/cut -d' ' -f1)
          nfc=$(${pkgs.coreutils}/bin/sha256sum ${nfc}          | ${pkgs.coreutils}/bin/cut -d' ' -f1)
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
        codegenCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-codegen-check";
          version = "0.0.0";
          src = codegenSrc;
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools pkgs.cargo-component ];
          buildPhase = ''
            runHook preBuild
            export RUSTC_BOOTSTRAP=1
            # codegenVendor is a symlinkJoin of 4 locks → merged = true (hand-rolled crates-io config).
            ${mkCargoVendorEnv { vendor = codegenVendor; merged = true; }}
            # xtask codegen --check regenerates runtime_abi.rs (building cdz-runtime + cdz-nfc components
            # via cargo-component to fold in their hashes) and fails if the committed file drifted.
            # Invoke the xtask binary via `cargo run --locked` (not the bare `cargo xtask` alias, which
            # omits --locked) so a root-lockfile drift is a HARD FAIL for this check (github-liaison
            # #2027/#2038 — no comparison to other checks; several deliberately omit --locked).
            cargo run --locked --package xtask --profile release -- codegen --check
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-codegen-check (cargo xtask codegen --check)" > "$out"
            runHook postInstall
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
        gateCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-gate-check";
          version = "0.0.0";
          src = gateSrc;
          # wasm-tools for the runtime↔program composition; codegenVendor is a superset root-lock vendor
          # (it also carries runtime/nfc/build-std locks, harmless here — gate builds only native host
          # binaries cdz/rcdzc/cdz-run, no build-std).
          nativeBuildInputs = [ rustToolchain pkgs.wasm-tools ];
          buildPhase = ''
            runHook preBuild
            # codegenVendor is a symlinkJoin of 4 locks → merged = true (hand-rolled crates-io config).
            ${mkCargoVendorEnv { vendor = codegenVendor; merged = true; }}
            # Grade the whole corpus against the committed baselines, resolving the runtime from my
            # nix-built component store (skips the CI job's `xtask build`). --locked = hard-fail on lock
            # drift (matches siblings).
            cargo run --locked --package xtask --profile release -- gate --check --store "${componentStore}"
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-gate-check (cargo xtask gate --check --store <nix store>)" > "$out"
            runHook postInstall
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
        benchCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-bench-check";
          version = "0.0.0";
          src = benchSrc;
          # codegenVendor is a superset root-lock vendor (also carries the cdz-runtime lock the bench test
          # compiles against). No wasm-tools/cargo-component: the bench test is a native host build.
          nativeBuildInputs = [ rustToolchain ];
          buildPhase = ''
            runHook preBuild
            export RUST_MIN_STACK=67108864
            # codegenVendor is a symlinkJoin of 4 locks → merged = true (hand-rolled crates-io config).
            ${mkCargoVendorEnv { vendor = codegenVendor; merged = true; }}
            # Runs cdz-runtime's hot_op_allocation_ceilings test + diffs the ALLOC counts vs
            # spec/bench/.alloc-baseline. --locked = hard-fail on lock drift (matches siblings).
            cargo run --locked --package xtask --profile release -- bench
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            echo "ok: cdz-bench-check (cargo xtask bench)" > "$out"
            runHook postInstall
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
        guideExamplesCheck = pkgs.stdenvNoCC.mkDerivation {
          pname = "cdz-guide-examples";
          version = "0.0.0";
          src = guideExamplesSrc;
          nativeBuildInputs = [
            rustToolchain
            wasmBindgenCli
            pkgs.binaryen # wasm-opt (the -Os shrink wasm-pack's --release applies)
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
            ${mkCargoVendorEnv { vendor = cdzWasmVendor; }}

            # ── 1. Build + bindgen the browser compiler wasm (the hermetic `wasm-pack build` equivalent).
            ( cd implementation/seed/crates/cdz-wasm
              cargo build --release --target wasm32-unknown-unknown --locked
              wasm-bindgen --target web --out-dir pkg \
                target/wasm32-unknown-unknown/release/cdz_wasm.wasm
              # wasm-pack's --release runs wasm-opt; the crate profile is opt-level="s" → -Os.
              wasm-opt -Os pkg/cdz_wasm_bg.wasm -o pkg/cdz_wasm_bg.wasm
            )

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
        packages.reducer-cadenza-b1-hash = hashOf reducerCadenzaB1 "reducer-cadenza-b1-hash";
        packages.reducer-cadenza-b2-hash = hashOf reducerCadenzaB2 "reducer-cadenza-b2-hash";
        packages.reducer-cadenza-b3-hash = hashOf reducerCadenzaB3 "reducer-cadenza-b3-hash";
        packages.reducer-cadenza-genesis-hash = hashOf reducerCadenzaGenesis "reducer-cadenza-genesis-hash";

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
            # seq-126 Part B: the 10 closure/tests-dir-scoped per-crate clippy+test checks (cdz is
            # workspace-src, crateCdzCheck below). In a let-attrset so BOTH the individual `crate-*` checks
            # AND the `clippy`/`test` AGGREGATE can reference them.
            perCrateChecks = {
              crate-cadenza-ast = mkCrateCheck { crate = "cadenza-ast"; };
              crate-cadenza-syntax = mkCrateCheck { crate = "cadenza-syntax"; extraSrc = [ ./spec/semantics ]; };
              crate-cdz-calc = mkCrateCheck { crate = "cdz-calc"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              crate-cdz-corpus = mkCrateCheck { crate = "cdz-corpus"; extraSrc = [ ./spec/semantics ]; };
              crate-cdz-num = mkCrateCheck { crate = "cdz-num"; extraSrc = [ ./implementation/seed/crates/cdz-runtime/src/bigint.rs ]; };
              crate-cdz-rt = mkCrateCheck { crate = "cdz-rt"; };
              crate-cdz-run = mkCrateCheck { crate = "cdz-run"; extraSrc = [ ./implementation/compiler-ml ]; };
              crate-cdz-rust-render = mkCrateCheck { crate = "cdz-rust-render"; };
              crate-rcdzc = mkCrateCheck {
                crate = "rcdzc";
                extraSrc = [ ./spec/semantics ./implementation/compiler-ml ./implementation/seed/crates/cdz-runtime/src/bigint.rs ];
              };
              crate-xtask = mkCrateCheck { crate = "xtask"; extraSrc = [ ./spec/semantics ./implementation/compiler-ml ]; extraInputs = [ pkgs.git ]; };
            };
            # cdz's check is WORKSPACE-SRC (concierge-confirmed 1a) — see the long note at its registration.
            crateCdzCheck = cargoWorkspaceCheck {
              name = "cargo-crate-cdz";
              cargoCmd = "cargo build --workspace --locked && cargo clippy -p cdz --all-targets --locked -- -D warnings && cargo test -p cdz --locked";
              src = seedTestSrc;
              extraInputs = [ pkgs.git ];
            };
            # `clippy`/`test` AGGREGATE: a thin node depending on ALL per-crate checks + crateCdzCheck.
            # RETAINED because the full-CI-in-nix cutover body-swaps the required GHA context `checks /
            # clippy` to `nix build .#checks.<sys>.clippy` — that NAME must resolve or the required context
            # reds (the #2130 reject). Builds nothing itself; forces every per-crate check, so `.#checks.
            # clippy` == "all crates lint+test clean" — KEEPS granularity (nix rebuilds only the per-crate
            # derivations whose inputs changed; this is a zero-cost dependency node) AND the CI contract.
            crateCheckAggregate = pkgs.runCommand "cargo-crate-aggregate"
              (perCrateChecks // { inherit crateCdzCheck; }) ''
              echo "ok: aggregate — all per-crate clippy+test checks built (seq-126 Part B)" > $out
            '';
            # crane MR2: the CLIPPY half via crane (per-crate cargoClippy consuming the shared cargoArtifacts →
            # deps NOT recompiled each run). Mirrors perCrateChecks' crate/extraSrc/extraInputs exactly. cdz
            # stays workspace-src (crateCdzCheck, different shape — its clippy is inside cargoWorkspaceCheck).
            # `checks.clippy` repoints here; `checks.test` stays on crateCheckAggregate until MR3 (test half).
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
            # crane MR3: the TEST half via crane (per-crate cargoTest consuming cargoArtifacts) — mirrors
            # perCrateClippyCrane's crate/extraSrc/extraInputs. cdz stays workspace-src (crateCdzCheck runs
            # `cargo test -p cdz` inside). `checks.test` repoints here; retires the old crateCheckAggregate.
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
            testCraneAggregate = pkgs.runCommand "cargo-test-crane-aggregate"
              (perCrateTestCrane // { inherit crateCdzCheck; }) ''
              echo "ok: test aggregate — all per-crate crane cargoTest checks + cdz (crane MR3)" > $out
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
            # seq-144: agent-harness bootstrap reducer @tests through nix (b1/b2/b3/genesis — 14 @tests).
            reducer-cadenza-tests = reducerCadenzaTests;
            # seq-144 Part 2: each B1-B4 reducer component is a valid wasm component (b3/genesis import kv
            # host-served/unresolved — validate checks STRUCTURE not import-satisfaction, so still green).
            reducer-cadenza-b1-valid = validComponent { name = "reducer-cadenza-b1"; drv = reducerCadenzaB1; };
            reducer-cadenza-b2-valid = validComponent { name = "reducer-cadenza-b2"; drv = reducerCadenzaB2; };
            reducer-cadenza-b3-valid = validComponent { name = "reducer-cadenza-b3"; drv = reducerCadenzaB3; };
            reducer-cadenza-genesis-valid = validComponent { name = "reducer-cadenza-genesis"; drv = reducerCadenzaGenesis; };

            # Full-CI-in-nix increment 1: the LINT pair, mirroring checks.yml `fmt` + `clippy` exactly.
            # `nix flake check` now runs them; the checks.yml jobs stay in place (advisory overlap) until
            # v-fleet-tooling's required-set cutover retires the hand-wired ones.
            fmt = cargoWorkspaceCheck {
              name = "cargo-fmt";
              cargoCmd = "cargo fmt --all --check";
            };
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
            # crane MR2: `checks.clippy` now = the CRANE clippy aggregate (per-crate cargoClippy consuming the
            # shared cargoArtifacts → deps compiled ONCE, only first-party src recompiles = the CI-throughput
            # win). `checks.test` STILL = crateCheckAggregate (the old clippy+test node) until MR3 rewires the
            # test half to crane; its redundant clippy is harmless short-term. Both context NAMES unchanged
            # (`checks / clippy`, `checks / test (ubuntu-latest)`) → no ruleset edit. Per-crate granularity +
            # isolation preserved (each crane check rebuilds only on its closure's src).
            clippy = clippyCraneAggregate;
            # crane MR3: `checks.test` now = the CRANE test aggregate (per-crate cargoTest consuming
            # cargoArtifacts → deps cached, only first-party recompiles = the test-ubuntu throughput win, same
            # as clippy got at MR2). Both required contexts now crane. Names unchanged → no ruleset edit. The
            # old crateCheckAggregate (clippy+test) is now UNREFERENCED (both checks.clippy + checks.test are
            # crane) — retiring mkCrateCheck/perCrateChecks/crateCheckAggregate is a follow-up cleanup (kept
            # this MR to the rewire; crateCdzCheck stays — both crane aggregates use it for cdz's workspace-src).
            test = testCraneAggregate;
            # Full-CI-in-nix increment 3: the native half of the GHA rcdzc-wasm job (the wasm build half
            # is the rcdzcWasm derivation / rcdzc-wasm-hash, already covered).
            rcdzc-wasm-native = rcdzcWasmNativeCheck;
            # Full-CI-in-nix increment 4: the GHA cdz-kernel job (test + clippy + fmt + live-exec).
            cdz-kernel-native = cdzKernelNativeCheck;
            # Full-CI-in-nix increment 5: the GHA cdz-agent-host job (test + clippy + fmt + feature matrix).
            cdz-agent-host-native = cdzAgentHostNativeCheck;
            # Full-CI-in-nix increment 6b: the GHA codegen job (cargo xtask codegen --check, ABI staleness).
            codegen-check = codegenCheck;
            # Full-CI-in-nix increment 6c: the GHA gate job (cargo xtask gate --check — THE behavior gate).
            gate-check = gateCheck;
            # Full-CI-in-nix increment 6d: the GHA bench job (cargo xtask bench — runtime alloc ceilings).
            bench-check = benchCheck;
            # Full-CI-in-nix increment 6e: the GHA cad-tests job (cdz test on the 4 in-tree Cadenza projects).
            cad-tests = cdzCadTestsCheck;
            # Full-CI-in-nix increment 6f: the GHA guide-examples job (the guide's runnable-content gate —
            # hermetic wasm-pack + npm ci + the check:* battery + build + bundle). The LAST required job.
            guide-examples = guideExamplesCheck;
            # Full-CI-in-nix increment 6a: the GHA `roundtrip` job — every corpus program round-trips
            # through the syntax surfaces. Corpus-only (reads spec/semantics, no runtime store) → narrow
            # `seedRoundtripSrc` (no compiler-ml, #2007). Invoked via `cargo run --locked` (not the bare
            # `cargo xtask` alias, which omits --locked) so a lockfile drift hard-fails, matching the
            # workspace test/clippy checks (#2032).
            roundtrip = cargoWorkspaceCheck {
              name = "cargo-xtask-roundtrip";
              cargoCmd = "cargo run --locked --package xtask --profile release -- roundtrip";
              src = seedRoundtripSrc;
            };
          }
          # seq-126 Part B: expose each per-crate check individually (granular signal + `nix flake check`
          # runs them). The `clippy`/`test` aggregates force this same set; these add per-crate cache
          # granularity + a precise red when one crate fails.
          // perCrateChecks;

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
