# Per-case nix caching for the corpus gate

Status: DESIGN (v-nix, 2026-08-25, operator-requested). Goal: stop re-running the whole
`spec/semantics/*.sexp` corpus on every change. Exploit nix content-addressing so a case only
re-runs when *its* inputs change, and — critically — **decouple compile from execute** so a
compiler change that does not alter a case's emitted artifact does not re-run that case.

## The idea

Shred each corpus file into one unit per `(case …)`, and run each case through a chain of nix
derivations whose keys are chosen so unrelated changes are cache hits:

```
corpus file ──shred──▶ per-case record ──build──▶ emitted artifact ──exec──▶ pass/fail ──▶ aggregate
             (parser)                    (compiler)                  (runtime, NO compiler)
```

- **shred** (one derivation per corpus file): parse the file, emit one record file per case.
  Key = `{corpus-file, shred-bin}`. Re-shreds only a *changed* file.
- **build** (one per case): compile the case's `input` program → emitted wasm (value-case) or the
  captured compile outcome / error-code (error-case). Key = `{case-record, compile-bin}`.
- **exec** (one per case): run the emitted artifact and grade against the case's `expect`
  (`(output (: v T))` | `(error CODE)` | `(trap …)` | `(declines)`). Key = **`{emitted-artifact,
  expect, exec-bin}` — the compiler is NOT an input.** So a compiler change that emits byte-identical
  wasm for a case leaves this derivation's inputs unchanged → nix reuses the cached result.
- **aggregate**: collect all exec results → suite verdict + counts.

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

- shred: `cdz corpus records FILE…` emits `---`-separated per-case records
  (`case\t… / call\t… / expect\t… / program\t… / host-calls\t… / wit-world\t…`). ADD an `--out-dir`
  mode writing one record file per case (one file = one nix input). Standalone bin: `cdz-corpus`.
- build: `cdz compile` (program → wasm). Expose a small `cdz-compile` bin.
- exec: `cdz run-emitted` (run a pre-compiled artifact — already compiler-independent). Add grading
  against an `expect` record (extend `cdz-run` or a small grader), so exec = one executable.

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

1. **shred `--out-dir`** on the `cdz-corpus` bin (per-case record files). Unit-tested.
2. **`cdz-compile` small bin** (compiler-only closure) — build a case record's program → wasm/outcome.
3. **`cdz-run` grade mode** — run-emitted + compare to expect → pass/fail.
4. **flake module**: `mkCorpusCase` (shred→build→exec) over ONE corpus file (01-literals) + aggregate;
   prove (a) compiler-comment change ⇒ exec 100% cache-hit, (b) one-case edit ⇒ only that case reruns.
5. Generalize to all 33 files; wire an aggregate `corpus` check into the flake.
6. (later) rust backend parallel exec layer; retire/relegate the monolithic xtask corpus path.

## Open/handled decisions

- Grading placement: fold into `cdz-run` (`--expect`) so exec is one bin (operator leans smaller bins;
  a standalone grader is also fine and even smaller-closure — decide at slice 3).
- Error/trap-cases grade compiler-dependently (at build) — accepted.
- wasm-only first — accepted.
