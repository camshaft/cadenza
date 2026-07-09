## 45. ✅ FIXED (seed, re-probed 2026-07-07) — a recursive effectful function on the runtime-compound path now lowers

**✅ RE-PROBED FIXED 2026-07-07.** The seed now emits the recursive-effectful runtime-compound path. Verified
against the current seed (all previously declined, now run):
- the exact diagnostics shape — recursion + `list`-state `Diag` (`emit`+`collect`), `(List.len (walk 3))` → **3**;
- recursion + resumed-value compound (`(resume (list v) s)`) → runs;
- `emit` performed from a NESTED helper under recursion → runs (→3);
- **at Core-walk scale**: recursing a nested-sum tree (`(KConst | KAdd (Tuple C C) | KBad)`) with `match`,
  performing `Diag.emit 201` at each `KBad`, collecting into a list → **2** (2 bad nodes) — exactly the shape
  `compiler.cdz`'s `well-typed?`/`resolve` walk needs;
- empty collect (no emits) → `0`.
So the seed's recursive-effectful lowering now covers the runtime-compound/host path, not just Stage-3 scalar.
This UNBLOCKS the operator's "effects in the compiler" direction for INTERNAL collection.

**⚠️ Remaining downstream blocker (NOT this ask): getting the collected diagnostics OUT through `compile`.** The
`Diag` effect can now COLLECT a `list<diagnostic>` across the recursive walk, but `compile` still can't RETURN
both bytes and diagnostics: a `Result` return (`(if … (Ok bytes) (Err diags))`) still declines "if branches
differ in kind" (ask-42 — the `Ok`/`Err` arms are different variant types), and the kinded-artifact record
return (`{artifacts, diagnostics}`, ask-41) still isn't decoded by the seed (reads `Ok (0 bytes)`). So the
diagnostics channel is now gated on ask-41 (realize the artifact ABI — a UNIFORM record return, no branch) or
ask-42 (fix the Ok/Err branch shape analysis). Effects handle everything up to the return; the return channel
is the last hop. See ask-42 / ask-41.

---
_Original finding (now resolved) below._

## 45. 🔴 A recursive effectful function whose effect touches a runtime compound (list/record) declines — blocks using effects in the compiler (diagnostics, and effects-everywhere)

**Direction (operator, 2026-07-07).** Lean on effects in the compiler: emit diagnostics via an effect, and use
effects broadly to clean up the pipeline. The natural diagnostics shape is a `Diag` effect with an `emit` op
and a `collect` (getter) op, threading a `list<diagnostic>` as handler state, performed from the recursive
program walk (`resolve`/`well-typed?`/`fold`) and handled at `compile`. This is also the ideal way to exercise
the effects-lowering path with a real workload.

**Finding.** The seed's effect lowering handles the recursive effectful path ONLY when it is "self-contained
scalar" (Stage 3). The moment recursion meets a **runtime compound** anywhere on the effect path — the handler
state is a `list`/record, OR an op's payload is a compound — it declines:
```
declined: recursive effectful function on the runtime-compound/host path not yet emitted
          (Stage 3 covers the self-contained scalar path)
```

**Boundary, isolated (2026-07-07, `cadenza-seed emit`) — three of four combinations WORK; the diagnostics one
does not:**

| shape | result |
|---|---|
| list-state handler, **non-recursive** body (`(do (D.emit 5) …)`) | ✅ runs |
| **scalar**-state handler (`(resume unit (+ s v))`), recursive walk | ✅ runs |
| getter/read-out op (`(D.get (u) s (resume s s))`), list-state, non-recursive | ✅ runs (the getter design is fine) |
| **list**-state handler (`(resume unit (List.push s v))`), **recursive** walk | 🔴 **declines** (above) |
| recursive walk, op **payload** = `(list …)`, even with scalar state | 🔴 **declines** (same) |
| recursive walk, **resumed VALUE** compound (`(resume (list v) s)`), scalar state | 🔴 **declines** (same) |
| **non**-recursive, list state AND list op payload together | ✅ runs |

So each ingredient works alone — list state ✅, recursion ✅, getter op ✅ — but **recursion + a runtime compound
touching the effect path** is the unimplemented case, and that is exactly diagnostics-collection (accumulate a
`list<diagnostic>` while recursively walking the program). Minimal repro (declines):
```
(module m
  (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (list Int64))))
  (def (walk n) (if (< n 1) (Diag.collect unit) (do (Diag.emit n) (walk (- n 1)))))
  (def (main)
    (handle (list)
       ((Diag.emit (v) s (resume unit (List.push s v)))
        (Diag.collect (u) s (resume s s)))
       (List.len (walk 3)))))
```
Control that runs (scalar state, same recursion): replace the list state with `0` and `List.push`/`List.len`
with `(+ s v)` → `ran → Value`.

**Why it matters.** This is the gate on the operator's "use effects in the compiler" direction, in two ways:
1. **Diagnostics via effects** needs a `list<diagnostic>` threaded through the effect while `resolve`/the
   type-checker recursively walk the program — precisely recursion + compound state. (It is also the clean fix
   for ask-42/ask-40: emitting diagnostics through an effect handled at `compile` avoids the Result-branch
   shape-analysis bug entirely — the body's value is the component, diagnostics ride the effect.)
2. **Effects-everywhere** (threading the symbol table / return-kind table / fresh-local counter / fold
   environment as effects instead of explicit accumulator args) all involve compound state carried across the
   compiler's deep recursion — the same shape, same wall.

The seed's effects-lowering learnings already classify handlers (tail-resumptive = inlined, abortive = block/br,
general one-shot = defunctionalized) and monomorphize the handler context; the remaining work is emitting a
recursive effectful function whose state/payload lives on the value heap (Perceus RC) rather than only in i64
scalars — the "runtime-compound/host path" the decline names as not-yet-emitted.

**Acceptance signal.** The minimal repro above runs (`List.len` of the collected list = 3), and more generally a
recursive function that performs an op carrying/accumulating a `list`/record compiles and runs (value-correct
vs the equivalent explicit-accumulator version). Then compiler.cdz can adopt a `Diag` effect for diagnostics
(and progressively for other threaded state), exercising the effects path with the compiler's real recursion.

**Ready-to-activate `Diag` design (the compiler-side wiring that will consume the fix; VERIFIED non-recursively
2026-07-07).** The exact structure compiler.cdz will adopt the moment this gap closes — I verified the
NON-recursive skeleton compiles + runs on the current seed (only the recursive `check` walk hits the wall):
```
(effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (list Int64))))
; the type-checker/reader walk performs (Diag.emit <CDZ-code>) at each rejection point instead of → KError
(def (check node) …recurse the Core…, performing (Diag.emit 201) on a type/arity error …)
(def (compile b)
  (handle (list)
     ((Diag.emit (v) s (resume unit (List.push s v)))   ; accumulate the diagnostic list
      (Diag.collect (u) s (resume s s)))                ; getter op = read out the collected list
     (do (check (resolve-module (read-module b)))
         <build the {artifacts, diagnostics} record from (compile-program …) and (Diag.collect unit)>)))
```
Non-recursive control (a straight-line `check`) runs today: `compile true` → 1 diagnostic collected, `compile
false` → 0 — VERIFIED `ran → Value`. This is ALSO why effects cleanly resolve ask-42: the `handle` body's value
is the component/artifact record, and diagnostics ride the `Diag` state — no `Ok`/`Err` branch for the seed's
shape analysis to choke on. So closing ask-44 unblocks the operator's effects direction AND the diagnostics
channel (ask-42/ask-40/ask-30 → `agree`) in one move.

**Status.** 🔴 Seed — the recursive-effectful lowering's runtime-compound/host arm. Scalar Stage 3 is landed;
this is the next stage. Related: the effects-lowering design + Stages-0–3 learnings, ask-42/ask-40 (diagnostics
channel — effects are the clean way to feed it), ask-13 (runtime-compound handling generally).
