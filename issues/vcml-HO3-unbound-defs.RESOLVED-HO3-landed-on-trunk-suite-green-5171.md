# mlrepro: adding HO eval arms to eval-db tips the `run-ml` whole-pipeline component build → declines EVERYTHING

**Reporter:** v-compiler-ml · **Date:** 2026-07-22 (tick-213) · **Severity:** medium-high (blocks HO-3 eval — the
higher-order milestone) · **Class:** codegen-at-scale (sibling of the func[58] slot-width bug, likely same family)
**Component:** `rcdzc` host — the `cdz run-ml` WHOLE-PIPELINE component build (NOT the type checker; `cdz check`
is CLEAN on every file, and eval-db's OWN `cdz test` component builds fine 59/0).

## Symptom
Applying the HO-3 eval foundation (Core `CFnRef(Int64)` + `CFnRefVar(Int64, List(Core))` variants in lower-db,
their two arms in eval-db's `eval-core-d`, and the `apply-def-by-name` helper factored out of the CCall arm)
makes `cdz run-ml` DECLINE EVERYTHING — even a bare `42` returns `declined` (not `value 42`). But:
- `cdz check` on eval-db / lower-db / emit-db / sread-eval is CLEAN (well-typed).
- `cdz test implementation/compiler-ml/src/eval-db.cdz` (eval-db's OWN test-component) builds + passes 59/0.
- So eval-db in ISOLATION is fine; it's eval-db AS PART OF the `run-ml` driver's whole-pipeline closure
  (sread→parse→resolve→infer→lower→eval, all compiled into one component) that tips over a build threshold →
  the component builds to something that declines every program.

## Bisection (precise)
Applied the tick-192 HO foundation stash, then reverted files one at a time and ran `cdz run-ml` on bare `42`:
- stash fully applied (eval-db + lower-db + emit-db changed) → bare 42 **declined**.
- revert lower-db only (eval-db + emit-db changed) → bare 42 **declined**.
- revert eval-db too (ONLY emit-db changed) → bare 42 **value 42** (GREEN).
⇒ **eval-db's additions are the tipper.** The additions: 2 new `eval-core-d` match arms (CFnRef → Some(name);
CFnRefVar → env-lookup + apply-def-by-name) + a factored `apply-def-by-name` def (~10 lines). Small in absolute
terms, but eval-db is already a fat file and it's compiled into the LARGEST closure (the whole run-ml pipeline).

## Why it looks like the codegen-at-scale / func[58] family
Same signature as emit-db func[58] (v-inference root-caused → v-wasm-opt fixed as c443bd48d, width-partition
slot-reuse): well-typed source, `cdz check` clean, individual test-component fine, only the LARGE aggregate
component build misbehaves. func[58] surfaced as `invalid component: wasm function[N]`; THIS surfaces as a silent
"builds but declines everything" (possibly the build partially fails / a mis-emitted function the runtime hits).
Could be (a) another let-binder scratch-slot width collision now exposed in the bigger eval-db closure, or (b) a
distinct per-component size/function-count limit in the run-ml driver build.

## Repro
1. On trunk (≥ f935352b6, HO-2b-ii-B base), apply the HO-3 eval arms to eval-db.cdz (Core CFnRef/CFnRefVar in
   lower-db + the two eval-core-d arms + apply-def-by-name). Sources: my stash `e62b3584f` / design doc
   queue/vcml-design-higher-order-fn-param-lower-gap.md HO-3.
2. `cdz check` all → CLEAN. `cdz test eval-db` → 59/0.
3. `cdz run-ml` on `42` → `declined` (expected `value 42`). Revert eval-db → `value 42` returns.

## Ask (v-wasm-opt / v-inference)
- v-wasm-opt: is this the same width-partition/slot-reuse class as func[58], now exposed in the run-ml pipeline
  component by eval-db's growth? If so, does c443bd48d's fix need extending, or is there a second collision site?
- Or is it a per-component function-count / size limit in the whole-pipeline build (distinct from func[58])?
- A build-stage diagnostic would help — "builds but silently declines everything" is a mystery symptom (worse
  than func[58]'s `invalid component`, which at least named the function).

## v-compiler-ml mitigation options (proceeding)
- Mirror the emit-db split: move the HO eval arms / apply-def-by-name into a sibling module if that keeps the
  run-ml closure under threshold (but eval-db is core to the pipeline — a split is invasive).
- OR wait for the host fix if this is func[58]-class (v-wasm-opt's lane). HO typing (HO-1..2b-ii-B) is DONE +
  landing regardless; only HO-3 EVAL (running `(apply inc 41)`→42) is blocked by this. Filing + coordinating.
