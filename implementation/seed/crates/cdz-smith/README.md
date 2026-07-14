# cdz-smith — a fuzzer for the reference compiler

`cdz-smith` generates Cadenza programs, feeds each through the real `rcdzc` compile path, and files
any program that makes the compiler **crash** (panic) or **hang** (timeout) as a runnable reproducer
for triage. It links the compiler as a library, so a rebuild against the latest `spec` is all it
takes to fuzz the newest compiler — no separate compiler build to manage.

```
seed bytes ──generate──▶ s-expr program ──parse+encode──▶ binary AST ──compile (catch_unwind)──▶ Verdict
                                                                                                    │
                                                                          Crash / Timeout ──────────┘
                                                                                  │
                                                                          shrink + dedup by site ──▶ spec/semantics/failures/
```

## Why a panic/timeout is a real bug

The compiler reports every legitimate "no" as **data** — a `Diagnostic` (a coded rejection) or an
uncoded decline ("not lowered yet"). It has zero `todo!`/`unimplemented!`. So on *any* input, the
only non-answers that are bugs are:

* a **panic** — an `.unwrap()`/`.expect(`/`unreachable!`/`panic!`/index/overflow firing, caught by
  `catch_unwind` around `compile_component` (which re-raises its worker-thread panic to us); and
* a **timeout** — a runaway loop, caught by a watchdog thread (an in-process catch can't interrupt a
  hang), which files the finding and aborts so the cron restarts.

A decline or a coded rejection is expected output and is never filed.

## Layout

| file | role |
|---|---|
| `src/generator.rs` | byte-seed → canonical s-expr program (depth/node-budgeted, always parseable) |
| `src/oracle.rs` | `compile_catching` — the crash oracle (panic hook + `catch_unwind`) |
| `src/finding.rs` | shrink to a minimal reproducer, dedup by crash **site**, write to the queue |
| `src/triage.rs` | convert libFuzzer crash/timeout artifacts → deduped findings |
| `src/driver.rs` | the PRNG-fallback loop + the hang watchdog |
| `src/bin/cdz-smith.rs` | the CLI (`fuzz` / `once` / `gen` / `verify` / `triage-artifacts`) |
| `tests/fuzz.rs` | the `bolero` property target — the coverage-guided engine's entry point |
| `fuzz-cycle.sh` | one cron cycle: sync spec → libFuzzer campaign → triage artifacts → findings |

cdz-smith is its **own workspace** (excluded from the seed workspace) so its `bolero` dependency
chain resolves independently and so `cargo bolero` can build it in isolation. Run cargo commands
from the crate directory, not with `-p` from the repo root.

## Engines

* **Coverage-guided libFuzzer (primary).** libFuzzer mutates a byte seed; `generate()` decodes it
  into a structured, always-parseable program; SanitizerCoverage feedback keeps inputs that reach
  **new compiler edges** and mutates them — so it climbs past the type-checker into the backend where
  the dense panic clusters live, and a **persistent corpus** accumulates that reach across runs.
  `-fork=1` isolates a crash/hang/OOM to one child and saves an artifact **without stopping** the
  campaign. Needs nightly + `cargo install cargo-bolero`.
* **PRNG driver (fallback).** `cdz-smith fuzz` — blind (no coverage), watchdog aborts on the first
  hang. No extra toolchain; same findings format. Used automatically when nightly/cargo-bolero
  are absent.

### Fork-mode phantom artifacts (expected, harmless)

Under `-fork=1`, libFuzzer occasionally saves a `crash-<hash>` artifact that is NOT a real fault: a
fork child killed by the outer backstop or by memory pressure (AddressSanitizer, cargo-bolero's
default, roughly triples RSS) is recorded as a "crash" against whatever tiny input it last logged.
These are recognizable — trivial inputs, correlated with low `exec/s`, and they do NOT reproduce
even under the same instrumented binary. `triage-artifacts` **replays every artifact and files only
those that reproduce**, so a phantom is counted and discarded, never filed. `fuzz-cycle.sh` minimizes
them by (a) letting libFuzzer exit on its own `-T` budget with a wide outer backstop, and (b) setting
`ASAN_OPTIONS` to disable the stack-use-after-return machinery that false-positives on the compiler's
hand-managed 64 MB guard-stack thread. rcdzc is pure safe Rust, so ASan on the compile path can only
false-positive anyway; it's kept solely because SanitizerCoverage links against its runtime.

## Use

```sh
cd implementation/seed/crates/cdz-smith    # it's its own workspace

# Coverage-guided campaign (the real thing): persistent corpus, fork-isolated, per-input timeout.
rustup run nightly cargo bolero test cdz_smith_never_panics \
    --engine libfuzzer -T 10m --timeout 10s \
    --corpus-dir /path/to/corpus --crashes-dir ./target/smith-crashes \
    -E-fork=1 -E-ignore_timeouts=1 -E-ignore_crashes=1 -E-ignore_ooms=1
# then turn the artifacts into findings:
cargo run -- triage-artifacts ./target/smith-crashes --findings <repo>/spec/semantics/failures

# PRNG fallback batch (no nightly needed).
cargo run --release -- fuzz --iterations 50000

# Inspect / reproduce a single seed (deterministic).
cargo run -- gen  1234        # print the generated program
cargo run -- once 1234        # generate + compile, print the verdict
cargo run -- verify a-finding.smith.sexp   # recompile a filed reproducer
```

## Findings

Each distinct crash **site** (normalized `file:line`, path-stable across checkouts) + masked message
template is one bucket: a `<sig>.smith.sexp` reproducer and a `<sig>.smith.md` note (category,
compiler commit, hit count, message + backtrace, a `verify` command). Re-hits bump the note's
counter and keep the smaller reproducer, so thousands of hits of one bug stay one file. The
`spec/semantics/failures/` queue is watched by the semantics-failures monitoring loop, which triages
and fixes; on resolution a note is renamed `.RESOLVED.md` / `.REJECTED.md` like the hand-written ones.

## Continuous operation

`fuzz-cycle.sh` is one cron cycle: it syncs a dedicated worktree to the latest `spec`, runs a
wall-clock-bounded **coverage-guided campaign** against a persistent corpus (falling back to the PRNG
driver when nightly/cargo-bolero are absent), then triages the artifacts into
`spec/semantics/failures/`. Point a 10-minute cron at it and the compiler is fuzzed continuously
against HEAD, always picking up new compiler builds, with coverage progress carried across cycles.

## Roadmap

* **Differential oracle** — compile a program to two backends (`Target::Wasm` and `Target::Rust`),
  run both, and flag any disagreement in the result value (a miscompile). The generator's kind-hint
  environment already biases toward well-typed programs that reach codegen; this is the next oracle.
* **Type-directed generation** — grow the `Env` seam so the generator emits mostly well-typed
  programs, reaching deeper past the type checker into the backend crash clusters.
