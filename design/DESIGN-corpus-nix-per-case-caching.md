# Per-case nix caching for the corpus gate

Status: DESIGN (v-nix, 2026-08-25, operator-requested). Goal: stop re-running the whole
`spec/semantics/*.sexp` corpus on every change. Exploit nix content-addressing so a case only
re-runs when *its* inputs change, and — critically — **decouple compile from execute** so a
compiler change that does not alter a case's emitted artifact does not re-run that case.

## The idea

Shred each corpus file into one unit per `(case …)`, and run each case through a chain of nix
derivations whose keys are chosen so unrelated changes are cache hits:

```
corpus file ─shred─▶ per-case artifacts   ─build─▶ emitted artifact ─exec─▶ pass/fail ─▶ aggregate
            (parser)  program/wit-world/    (compiler)               (runtime, NO compiler)
                      component-name/test-run
```

- **shred** (one derivation per corpus file): parse the file, emit per case the artifacts each already in
  its CONSUMER's NATIVE form — the shred does every format transform ONCE here so no consumer re-converts
  (the operator's "fewer transforms the better" steer). Binary-AST artifacts are `cadenza_ast::codec::encode`
  of the parsed form (we already parse the `.sexp` — no third text format). Split BY CONSUMER (SHIPPED,
  `cdz corpus records --out-dir`):
  - `program.ast` (+ `module-<name>.ast` per package sibling) — the program(s) as binary AST, the compiler's
    native `ast:<name>=…` input.
  - `wit-world.ast` — the imposed world as binary AST (the `(world …)` subtree ITSELF, which already IS the
    `world_schema_tree` shape rcdzc reads), the compiler's native `wit-world:<name>=…` input verbatim (the
    `<name>` label is ignored — the world name is read from the artifact); omitted for the common
    synthesized-world case.
  - `component-name` — the interface the world's guest exports under, as PLAIN TEXT (a `--component-name`
    string, not an AST); omitted unless the case names one.
  - `test-run.ast` — description + trials (call/args/expect) + host-calls/responses + warns, the
    **runner/grader** metadata; NOT a compiler input.
  Key = `{corpus-file, shred-bin}`. Re-shreds only a *changed* file. A `manifest` per file lists the case
  dirs so nix enumerates cases (tiny IFD) without re-parsing the `.sexp`.
- **build** (one per case, per backend): compile `{program.ast (+modules), wit-world.ast?, component-name?}`
  → emitted wasm (value-case) or the captured compile outcome / error-code (error/declines-case). Because
  every input is already the compiler's native form, `cdz-compile` is a pure passthrough (no decode/reencode).
  Key = `{program.ast, modules, wit-world.ast, component-name, compile-bin}` — a **run-metadata** edit
  (expected output, args, host tape → `test-run.ast`) is NOT a build input, so it never rebuilds.
- **exec** (one per case, per backend): run the emitted artifact and grade against `test-run.ast`
  (`(expect-output …)` | `(expect-error CODE msg?)` | `(expect-trap …)` | `(expect-declines msg?)`,
  per-trial, + host tape + warns). Key = **`{emitted-artifact, test-run.ast, exec-bin}` — the compiler is
  NOT an input.** So a compiler change that emits byte-identical wasm for a case leaves this derivation's
  inputs unchanged → nix reuses the cached result.
- **aggregate**: collect all exec results → suite verdict + counts (per case × backend).

## Why smaller binaries (operator preference — and it sharpens caching)

Each phase runs a dedicated binary with a MINIMAL dependency closure, so each derivation re-runs only
when *that phase's* code changes — not on any unrelated `cdz` change (the monolithic `cdz` binary
rotates on ANY subcommand edit, which would invalidate every derivation keyed on it):

- **shred-bin** = the existing standalone `cdz-corpus` bin. Closure = corpus parser (cdz-corpus +
  cadenza-syntax). Rotates only on a parser change.
- **build-bin** = a NEW small `cdz-compile` bin (rcdzc has no standalone bin today; compile only runs
  via the unified `cdz`). Closure = compiler. Rotates only on a compiler change.
- **exec-bin** = the existing `cdz-run` bin (+ a grade mode). Closure = runtime + grader, **excluding
  the compiler**. A compiler change CANNOT invalidate the exec layer (beyond the artifact input) — this
  is what makes the build/exec decoupling airtight.

This mirrors the harness framework's already-landed `mkHarnessAst` (transform) vs `mkHarnessRun`
(execute) decoupling (operator review on #3299), generalized to the corpus with per-case granularity.

## Phase primitives (mostly exist as `cdz` subcommands; expose as small bins)

- shred: `cdz corpus records --out-dir DIR FILE…` writes one per-case dir of native-form artifacts
  (`program.ast` / `module-*.ast` / `wit-world.ast` / `component-name` / `test-run.ast`) + a `manifest`
  (SHIPPED). Standalone bin: `cdz-corpus`.
- build: `cdz compile` already takes binary-AST `ast:/wit-world:` inputs + `--component-name`/`--entry`
  and emits per-backend (`-t wasm|rust|…`); `rcdzc::cli::parse_and_run()` is the standalone entry.
  Expose it as a small `cdz-compile` bin (rcdzc has a `[lib]`, no `[[bin]]` yet) — a compiler-only-closure
  passthrough, since the shred already hands it native inputs.
- exec: `cdz run-emitted` (run a pre-compiled artifact — already compiler-independent). Add grading
  against `test-run.ast` (extend `cdz-run` or a small grader), so exec = one executable.

## Cases the design must handle

- **value-case** `(output (: v T))`: build → wasm; exec → run + compare value-form. exec compiler-independent.
- **error-case** `(error CODE)`: no runnable artifact — the compile OUTCOME is the test; graded at
  build (correctly compiler-dependent; a compiler change can change the diagnostic → reruns).
- **trap-case** `(trap "reason")`: build → wasm; exec → run, expect a trap with matching reason.
- **declines** / **warns** / **multi-trial** `(call …)(output …)` / **host-calls** (host-response tape)
  / **wit-world** (imposed world) / **package** (sibling library modules): the record already carries
  these fields; exec replays them. Port faithfully from the current xtask/cdz-corpus runner.
- **backends**: start wasm-target only (mirrors the harness). Rust backend = a parallel exec layer later.

## Faithfulness

The nix derivations must reproduce EXACTLY what `cargo xtask gate` does today (same compile flags, same
run, same grading + baseline compare), so this is a cache-friendly re-hosting, not a behavior change.
The existing `xtask gate` stays as the authoritative fallback until the nix path is proven equivalent
on the full corpus.

## Rollout (incremental, each slice gated + landable)

1. **shred `--out-dir`** on the `cdz-corpus` bin — per-case dirs of NATIVE-form artifacts
   (program/module/wit-world binary AST + component-name text + test-run), every transform done once at
   shred (DONE: #3364 + #3382 single-form program root; native wit-world/component-name replacing the
   compile-unit container). Unit-tested; verified end to end that a shredded case (incl. an imposed
   `wit-world.ast` + `component-name`) compiles with the compiler's native args and NO transform.
2. **`cdz-compile` small bin** (compiler-only closure) — a passthrough over `rcdzc::cli::parse_and_run()`
   building a case's native artifacts → wasm/outcome per backend.
3. **`cdz-run` grade mode** — run-emitted + compare to expect → pass/fail.
4. **flake module**: `mkCorpusCase` (shred→build→exec) over ONE corpus file (01-literals) + aggregate;
   prove (a) compiler-comment change ⇒ exec 100% cache-hit, (b) one-case edit ⇒ only that case reruns.
   DONE (#3407): per-case shred → content-addressed build → compiler-free exec; CA store; (a) PROVEN.
5. Generalize to all 33 files; wire an aggregate `corpus` check into the flake. DONE (#3407): top-level
   `corpus` + per-file `corpus-<file>`; the exec grader now reproduces the gate for the wasm target,
   including exact error-code/message + `warns` + host-call sequence (#3418).

## Retiring the `xtask gate` — the three remaining parity gaps

The nix `corpus` check is now a faithful WASM-target replacement (value/trap/host-calls/error-code/warns,
Todo-on-declined-value-case like the gate). To DELETE `xtask gate` + `.gate-baseline*` + the gate code,
three gaps remain:

6. **Rust + rust-async exec layer** (the biggest — the gate runs every case through `rust`/`rust-async`
   too, each its own ~7200-case baseline; the nix graph is wasm-only). Shape (from `xtask`
   `run_program_rust`): emit `.rs` (`cdz-compile -t rust[-async]`, already supported) → generate a DRIVER
   (export call + type-aware arg marshal incl. `cdz_num::Big`, host-response shim fns, factory/closure
   application) → `rustc` linking the pre-built rlibs (`cdz_rt` for the async `CdzEnv`, `cdz_num` for
   `Big`, `cadenza_ast` for native R2; via `-L dependency=<dir> --extern <crate>=<rlib>`) → run → grade.
   That driver-gen + rustc-run logic lives ONLY in `xtask` today (which is bloated) — so EXTRACT it into a
   DEDICATED CRATE `cdz-rust-run` (operator 2026-08-26: "make a dedicated crate for the rust runners; the
   xtask is just getting so bloated"): the crate holds the rust driver-gen + rustc-invoke + run + grade
   (reusing the shared `cdz-corpus-grade` compare for the outcome), exposes a `cdz-rust-run` bin the nix
   per-case rust exec layer invokes, and xtask's `run_program_rust` becomes a thin call into it (so the
   logic has ONE home and xtask sheds weight). Plus a nix derivation that pre-builds the rlibs ONCE (CA) +
   a per-case rust build+exec layer mirroring the wasm one. Multi-tick.

   STATUS: the crate is BUILT — `sig` (signature analysis), `driver` (export call + host-response shims),
   `run` (rustc compile+run, linking the caller-supplied `cdz_rt`/`cdz_num`/`cadenza_ast` rlibs), and
   `grade` (the rust trial-runner through `cdz-corpus-grade`), plus the `cdz-rust-run --grade` bin the nix
   layer shells out to. REMAINING: the nix CA-rlib derivation + per-case rust build+exec layer (mirror the
   wasm `mkCorpusBuild`/`mkCorpusExec`), then wire xtask's `run_program_rust` to delegate at cutover. The
   host-closure factory/consumer application + the async `block_on` harness (the small closure/async corpus
   subset) are deferred — those cases decline-to-Todo until added.
7. **Regression detection (baseline-diff)**: the gate diffs each case's verdict against `.gate-baseline`
   (7225 pass + 40 todo + 0 fail), so a `Pass→Todo/Fail` REGRESSION fails it; the nix graph only catches
   an outright Fail (a declined value-case is Todo, exit 0). Options: (a) port a committed per-case
   baseline the exec diffs against, or (b) encode the expected-Todo set in the corpus SOURCE (a `(todo)`
   annotation on the ~40 known-incomplete cases) so unmarked cases MUST pass — no separate baseline file.
8. **Cutover**: once 6+7 land and the nix `corpus` (wasm+rust+rust-async) is proven equivalent on the
   full corpus, DELETE `xtask gate`, `.gate-baseline*`, and the gate/grader code from `xtask/src/main.rs`.

## Local-emit test loop (which path picks up a LOCAL rcdzc edit)

Two grade paths exist; they build the compiler DIFFERENTLY, so an agent iterating on a `rcdzc` emit
change must know which reflects a local (uncommitted) source edit:

- **`cargo xtask gate` / `cargo xtask emit`** — DO reflect a local `rcdzc` edit. Both call `build_tools()`,
  which runs `cargo build --profile release-debug -p cdz …` from LOCAL source and then shells that
  freshly-built `target/release-debug/cdz` (`tools.rcdzc = cdz`). `cdz`'s default features include
  `standalone`, so `cdz compile` runs the compiler IN-PROCESS (`compiler_cli::run`); the `delegate` module
  and its `CDZ_COMPILE_BIN` env read are `#[cfg(not(feature = "standalone"))]` — compiled OUT of the
  cargo-built binary (verified: a bogus `CDZ_COMPILE_BIN` is ignored by the standalone `cdz`). So the gate
  never uses the nix / content-addressed compiler, and a `cargo build -p cdz` recompiles `rcdzc` into `cdz`.
- **`nix build .#checks.<sys>.corpus-*`** (this pipeline) — builds the compiler in a content-addressed nix
  derivation from COMMITTED source. An UNCOMMITTED local edit is invisible here until committed; once
  committed, the CA build reruns only for the cases whose emit changed (that is the whole point).

So the sanctioned loop for a `rcdzc` emit edit — no commit needed:

1. SEE the emit: `cargo xtask emit <file.sexp>` (or `cdz convert <f> -t binary | cdz compile - --target rust -o -`).
2. GRADE it: `cargo xtask gate --target rust --files <file>` (whole file), or
   `cargo xtask gate --target rust --case <needle>` (single-case debug loop: prints program/expected/actual).

Run from the SAME worktree the edit lives in. NOTE: a full `xtask gate` run prints only a pass/fail tally
and captures per-case compiler stderr (surfaced only on a decline/reject as the diagnostic; discarded on a
clean compile), so an instrumentation `eprintln!` on the SUCCESS path fires but is not shown — use `--case`
or `xtask emit` to see per-case detail. The rust gate is also DIFFERENTIAL (grades the rust emit against the
wasm oracle), so a correctness-preserving emit fix will not flip a verdict.

## Open/handled decisions

- Grading placement: fold into `cdz-run` (`--expect`) so exec is one bin (operator leans smaller bins;
  a standalone grader is also fine and even smaller-closure — decide at slice 3).
- Error/trap-cases grade compiler-dependently (at build) — accepted.
- wasm-only first — accepted.
