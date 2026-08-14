# 3-way-partition — F24-class arm-code duplication (DISTINCT from sft1 scrutinee-dup)

Routed by corpus-bugfix (issue 2026-08-14). Tested on my sft1-fix-carrying cdz
(HEAD 17e3ed857, the scrutinee-hoist fix built in).

> **⚠️ CORRECTION (2026-08-15, after v-rb co-gate of the emit-budget fix).** "EXPONENTIAL"
> below describes the emit-SIZE GROWTH RATE (~6^N), which is real. It does NOT mean every
> K>=3 case is INVALID wasm. VALID-LARGE ≠ INVALID:
> - **VALID-but-large (load + RUN fine):** isolate-K3 (593 KB, runs → 201202203204205301),
>   isolate-K4 (215 KB, runs → 200400300301302), and the corpus high-water cbk1 (~416K Lir)
>   / sw4 / sw5. Large emitted bodies, but the engine loads + runs them.
> - **GENUINELY INVALID (engine rejects the module):** dst1 (2.88 MB → cranelift "Code for
>   function is too large"), dstC (166 MB → "function body size count exceeds 7654321"),
>   scn1 (too-many-locals). These are the soundness bug.
> The `isolate-K3/K4-...-EXPONENTIAL.sexp` filenames name the GROWTH, not invalidity — K3/K4
> are the valid-large tail that must KEEP compiling; dst1/dstC/scn1 are the invalid tail the
> emit-budget decline (`EMIT_INSTRUCTION_BUDGET=1_000_000` Lir, v-rb `9c55a11ab`) rejects
> cleanly. The 1M-Lir bound sits >2x over the ~416K valid high-water and under dst1's ~1.48M.

## Both faces are F24-class code-SIZE explosion (super-linear emit per dispatch)
FACE-1 (dst1/dstB): 3-tuple state, `(% n 4)` pivot recomputed in the arm.
  dstA(5 disp): 321354 B, RUNS (=101201102301211).
  dst1(6 disp): 2883376 B, cdz-run "Code for function is too large".
  ~9x emit growth per +1 dispatch.
FACE-2 (dstC/dstD/dstE): pivots carried in a 5-WIDE tuple state, no n in arm.
  dstE(4): 550990 B PASS.  dstD(5): 9647120 B FAIL.  dstC(6): 165963565 B FAIL.
  ~17x per +1 dispatch. wasm-tools error KIND (corpus-bugfix asked):
  "function body size count exceeds limit of 7654321" (a code-SIZE limit, at func[7]),
  NOT too-many-locals. Wider state => lower dispatch threshold (per-dispatch dup).

## NOT covered by the sft1 scrutinee-hoist fix
- dst1 with my fix: still 2883376 B, still "too large" (my fix fires only on a
  single BARE-NAME binder over a COMPOUND scrutinee; these arms match `st` (a var)
  with a TUPLE pattern — my gate declines).
- Hoisting `(% n 4)` to a `let` OUTSIDE the handle: still 3107534 B, still fails.
  So the dup is NOT the seed-expression / captured-free-var.

## Mechanism (distinct F24 sibling): MULTI-RESUME-POINT continuation splice
These arms have K=3 resume points (a nested `if` with 3 `(resume …)` tails). The
resumptive fold splices the continuation C at EACH resume site; each spliced C
contains the NEXT dispatch's arm, again with 3 resume points → ~K^(remaining
dispatches) copies of the arm CODE. That is the super-linear/exponential emit.
- sft1 was SCRUTINEE duplication (locals-count limit) — fixed by hoisting the
  compound scrutinee to a let.
- THIS is ARM-CODE duplication from multiple resume points (code-SIZE limit) — a
  DIFFERENT gap. The fix must SHARE the continuation across the K resume sites
  (join block / single continuation function) instead of splicing a fresh copy per
  resume, so K resume points cost K branches into one shared continuation, not K
  copies. Harder than the scrutinee-hoist; own full-battery cycle.

## PROVEN isolation ladder (2026-08-14, next tick) — resume-point-count IS the multiplier
Clean non-folding ladder (dispatch results consumed in a product; branch condition
references the runtime param `m` so nothing constant-folds), emit size vs #dispatches:

  R1 single resume point, tuple state, runtime branch:  159 182 206 231  (N=3..6) LINEAR
  R3 K=3 resume points,  tuple state, runtime branch:  3637 17326 99701 593123 EXPONENTIAL (~5-6x/disp)
  k3 K=3 resume points,  SCALAR state, runtime accum:  587 790 993 1196  LINEAR (control)

CONCLUSIONS:
- The multiplier is the RESUME-POINT COUNT K when the taken branch is NOT statically
  decidable (R1=linear, R3=exponential; both same tuple state + runtime branch).
- Multiple resume points ALONE is not enough (k3: K=3 with a scalar accumulator is
  LINEAR). The explosion needs K>1 resume points AND a per-dispatch-growing state the
  continuation threads (the tuple state reconstructed per resume).
- Mechanism CONFIRMED: the fold splices the continuation C at EACH of the K resume
  sites; each spliced C carries the NEXT dispatch's K-resume arm -> ~K^(remaining
  dispatches) copies of the arm CODE. Face-1/face-2 are K=3 arms.

Probes: isolate-R1-single-resume-...-LINEAR.sexp, isolate-R3-three-resume-...-EXPONENTIAL.sexp,
isolate-k3-scalar-state-...-LINEAR-control.sexp (in this bank).

FIX (unchanged): share the continuation across the K resume sites (a join block /
single continuation function reached by K branches) instead of splicing a fresh copy
per resume. Distinct from sft1's scrutinee-hoist. Own full-battery cycle. DEFERRED
until the sft1 MR 17e3ed857 lands (branch frozen behind the queued MR).

## Expected outputs (from the .sexp cases)
dst1 main(10)=101201102301202221, main(0)=201301101302202122
dstC main(10)=101201102301202203
