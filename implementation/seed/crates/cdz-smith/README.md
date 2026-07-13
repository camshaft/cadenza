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
| `src/driver.rs` | the continuous loop + the hang watchdog + the run PRNG |
| `src/bin/cdz-smith.rs` | the CLI (`fuzz` / `once` / `gen` / `verify`) |
| `tests/fuzz.rs` | a `bolero` property target (shrinking + coverage-guided, `#[ignore]` by default) |
| `fuzz-cycle.sh` | one cron cycle: sync spec → rebuild → time-boxed batch → findings to the queue |

## Use

```sh
# Fuzz a batch; findings land in spec/semantics/failures/ (auto-discovered).
cargo run -p cdz-smith --profile release-debug -- fuzz --iterations 50000

# Inspect / reproduce a single seed (deterministic).
cargo run -p cdz-smith -- gen  1234        # print the generated program
cargo run -p cdz-smith -- once 1234        # generate + compile, print the verdict
cargo run -p cdz-smith -- verify a-finding.smith.sexp   # recompile a filed reproducer

# The bolero property (explicit; no in-process hang guard — see the test's comment):
cargo test  -p cdz-smith --test fuzz -- --ignored          # bounded random
cargo bolero test cdz_smith_never_panics -p cdz-smith      # coverage-guided (nightly)
```

## Findings

Each distinct crash **site** (normalized `file:line`, path-stable across checkouts) + masked message
template is one bucket: a `<sig>.smith.sexp` reproducer and a `<sig>.smith.md` note (category,
compiler commit, hit count, message + backtrace, a `verify` command). Re-hits bump the note's
counter and keep the smaller reproducer, so thousands of hits of one bug stay one file. The
`spec/semantics/failures/` queue is watched by the semantics-failures monitoring loop, which triages
and fixes; on resolution a note is renamed `.RESOLVED.md` / `.REJECTED.md` like the hand-written ones.

## Continuous operation

`fuzz-cycle.sh` is one cron cycle: it syncs a dedicated worktree to the latest `spec`, rebuilds
`cdz-smith` against that compiler, and runs a wall-clock-bounded batch, writing findings to the main
checkout's `spec/semantics/failures/`. Point a 10-minute cron at it and the compiler is fuzzed
continuously against HEAD, always picking up new compiler builds.

## Roadmap

* **Differential oracle** — compile a program to two backends (`Target::Wasm` and `Target::Rust`),
  run both, and flag any disagreement in the result value (a miscompile). The generator's kind-hint
  environment already biases toward well-typed programs that reach codegen; this is the next oracle.
* **Type-directed generation** — grow the `Env` seam so the generator emits mostly well-typed
  programs, reaching deeper past the type checker into the backend crash clusters.
