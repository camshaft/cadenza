# DESIGN (v-compiler-ml, self): SPLIT the heavy compiler-ml test files — the documented "throughput-drag red gate" in-lane fix

Scoped 2026-07-21 (base 01ad6b9b6-era; param4 resend e24228335 pending). xtask/src/main.rs (suite_timeout_for,
lines 617-627) EXPLICITLY names this as "v-compiler-ml's file SPLIT ... their in-lane fix — shrinks the per-file
build so no file nears the cap". The compiler-ml sweep is "the dominant sweep" / "standing throughput-drag red
gate": each run-src @test compiles a Cadenza program through the ML compiler AND runs it under wasmtime, and the
suite is given a 45-min cap (vs 6-min default) because the heaviest file measured >300s under load.

## The heavy files (measured this tick during a live batch-gate bake — all at ~98% CPU)
- `conformance-db.cdz`: **77 @tests**, 1079 lines — each `conformance-*` @test calls check-case → run-tokens-db
  → the FULL memoized Db pipeline. The single biggest file. (ran first in the observed batch gate.)
- `sread-eval.cdz`: 41 run-src @tests, 390 lines — xtask docs call out THIS one as ">300s under load".
- `sread-eval-fns.cdz`: 31; `sread-eval-ann.cdz`: 27 — also heavy run-src.

## Split plan (per file, mechanical + safe)
conformance-db already EXPORTS its engine: `export { Expect.*, Case.*, case-passes, check-case, count-passing,
corpus, conformance, all-pass }`. So a SECOND file can `import { Expect, Case, check-case, ... } from
"conformance-db"` and hold half the @tests. Split:
- Keep `conformance-db.cdz` = the engine (types, case-passes/check-case/count-passing/corpus/conformance) + the
  first ~38 conformance-* @tests (literal..bool-div-declines, the basic-arith/bool/if/let/div block).
- New `conformance-db-cx.cdz` = `import` the engine + the ~39 `conformance-cx-*` @tests (the composition block,
  lines ~1010-1072). These are the natural second half (all named `conformance-cx-*`).
This roughly HALVES conformance-db's per-file build+run, and the per-file PARALLEL gate sweep
(step_cached_per_file, HOW_MANY_CONCURRENT) runs the two halves concurrently.

## Expected benefit (measured-nuance, don't oversell)
During the CPU-SATURATED overlap (I observed 4 files each at ~98% CPU simultaneously), splitting one file into
two does NOT speed the batch — the cores are already full. The real wins:
1. **Per-file cap safety**: no single file nears the 12-min per-file / 45-min suite cap → kills the
   nondeterministic false-HANG-fail on load spikes (the stated red-gate cause).
2. **Tail latency**: when the other files finish, the last big file runs alone on idle cores; halving it halves
   that tail.
3. **Bisectability**: a failure localizes to a smaller file.
LOG this nuance — do NOT claim it "halves the suite"; it de-risks the cap + trims the tail.

## Safety (verified this tick)
- conformance-db.cdz + sread-eval.cdz: **0 trunk commits touched them since my base** (git log base..trunk --
  <file> = 0) → splitting on my behind-base has NO stale-base conflict.
- NOT in my pending param4 MR's file set (infer-db/lower-db/resolve-db/sread-eval-fns/sread) → no content overlap.

## ⚠ Why not now (the BLOCKER): the stacking trap
My param4 resend (e24228335) is UNLANDED and is the only commit between trunk and HEAD. A split commit here would
STACK on it — sending the split's --ref would carry param4's diff (parent = unlanded param4) → pr-sync double-
applies or conflicts. And sync REFUSES to rebase (would orphan param4's queued --ref). So NO code MR of any kind
can land until param4 clears trunk. Once it lands: `cargo xtask sync --force`, then this split is a clean
standalone commit (touches only conformance-db.cdz + the new conformance-db-cx.cdz). Gate: full compiler-ml suite
still green (same total, redistributed across 35 files); confirm the new file's @tests all run + the engine
imports resolve.

## Priority ordering once param4 lands
1. item-3 HM (deferred-int) — biggest CONFORMANCE win, fully scoped+de-risked
   (vcml-design-item3-hm-deferred-int-infer-boundary).
2. this file-split — biggest FLEET-THROUGHPUT win (unblocks the standing red-gate drag), independent of #1
   (different files: #1=infer-db/ty-bridge, #2=conformance-db).
3. forward-ref/recursion — biggest FEATURE win but largest+atomic
   (vcml-design-forward-ref-recursion-napp-carries-name).
All three are execute-ready; none can start until param4 clears (all touch the compiler-ml package on a base I
can't sync while the MR is queued).
