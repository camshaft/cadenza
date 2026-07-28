# DESIGN (v-compiler-ml, self-authored): Slice B — true runtime recursion via a non-inlining Core call form

Owner: v-compiler-ml. Status: PLANNED, execution-ready. Blocked ONLY on landing MR `dc204e163`
(forward-refs + main-anywhere) — do NOT start until trunk contains it and the base is clean (a new feature
commit must not stack on the pending MR). Supersedes the high-level sketch in
`vcml-followup-recursive-user-functions-decline.md` (§UPDATE 2026-07-20) with a concrete arm-by-arm plan.

## Where we are after forward-ref (the base this builds on)
- `NApp(calleeName, argId)` carries the callee NAME-id; resolve/infer/lower map name→bodyId via `def-body-of`.
- A call is handled by INLINING the callee body's Core (nullary: inline body; param: `CLet(param, arg, body)`).
- A RECURSIVE call (body transitively calls back, detected by `call-is-recursive`, parse-db.cdz:160) currently
  **DECLINES**: infer→TErr (infer-db.cdz:105), resolve→no body descent (resolve-db.cdz:88), lower→None
  (lower-db.cdz:117). This is honest coverage-not-yet — inlining a cycle would loop forever.
- Core IR (lower-db.cdz:23): `CNum | CVar | CBin | CLet | CIf` — NO call/fix node. `eval-core(c, env)`
  (eval-db.cdz:22) carries only `env: Map(binderId, Int64)` — NO function values. So recursion cannot be
  expressed today; it MUST inline, and a cycle can't inline. That is the whole reason for Slice B.

## The feature
Make `(def (fac n) (if (< n 1) 1 (* n (fac (- n 1))))) (fac 5)` RUN to 120 (currently declines). Also
`countdown`, `sum`, mutual `ev/od`. Reference is definition-order- AND recursion-independent; this closes the
last big self-host conformance gap (recursion is pervasive in the real corpus: fac/sum/fib).

## Design: a non-inlining Core CALL node + a def-env in eval (interpreter first, emit second)

### B1 — Core gets a real call node (lower-db.cdz `type Core`)
Add `CCall(Int64, List(Core))` — `CCall(calleeName, args)`: a call to a def by NAME (not inlined), args
already-lowered Core. Keep the existing inline path for NON-recursive calls (cheap, no env-of-defs needed) and
emit `CCall` ONLY when `call-is-recursive` is true. Rationale: minimal blast radius — non-recursive forward/
backward refs keep inlining (proven green); only true cycles switch to the call node. (Alt considered: always
emit CCall — REJECTED, would regress the 100+ inline @tests and needs a def-env even for straight-line code.)

- lower-db NApp arm (lower-db.cdz:117): the `call-is-recursive` branch currently returns `Option.None(unit)`.
  CHANGE it to: lower each arg (argId, arg2-of..arg4-of by the SAME param/arg machinery already there) → build
  `Option.Some(Core.CCall(calleeName, [arg1Core, arg2Core, ...]))`. Nullary recursive call → `CCall(name, [])`.
  Unknown-name still declines (def-body-of None → None), unchanged.
- ✅ **DESIGN RISK RESOLVED (2026-07-22 probe): the ML surface FULLY SUPPORTS a recursive sum with a
  `List(Self)` field — `CCall(Int64, List(Core))` is SAFE, no fallback needed.** Although NO existing
  compiler-ml sum has a self-referential List field (grep), a throwaway probe PROVED the language handles it:
  `type Tree = Leaf(Int64) | Branch(List(Tree))` + `sum-tree`/`sum-list` mutual recursion COMPILES (`cdz
  compile` → 1351-byte wasm) and RUNS correctly (`Branch([Leaf 3, Leaf 4, Branch([Leaf 5])])` → 12). So B1's
  `CCall(name, List(Core))` needs no flat-field workaround; use the clean `List(Core)` (arbitrary arity). (The
  earlier proposed fallback — flat `CCall(name, Core, Core, Core, Core)` with sentinels — is NOT needed; kept
  here only as a note that it existed.) B1a can proceed straight to `List(Core)`.
- ✅ **B3 RECURSION MECHANISM DE-RISKED (2026-07-22 tick 17 probe): the FULL CCall-through-def-env
  interpreter arm RUNS in the self-host language — `fac(5) → 120`.** A standalone probe mirrored the exact
  planned arm: `eval(MCall(name,args), env, defs)` → `Map.lookup(defs, name)` → destructure
  `Some((params, body))` → `eval-args` (each arg in the CURRENT env) → `bind-params` positionally into a
  FRESH env → re-enter `eval(body, freshEnv, defs)`, terminating on the program's own `MIf` base case (NO
  fuel). Compiled (2564-byte wasm) + ran → 120. So the recursion-evaluation MECHANISM (not just the two
  type-shapes) is proven — B3's `eval-core` CCall arm can be written as designed with no fallback. TWO
  self-host idioms confirmed for B3: (i) a `List` MUST be consumed via `List.at`/`List.len` INDEX recursion —
  cons patterns `p :: rest` do NOT parse (matches parse-db `tok-at`, conformance-db); the compiler-ml sources
  never cons-destructure. So `eval-args`/`bind-params` in B3 use index recursion, not head/tail. (ii) The
  `Map(Int64, (List(Int64), Core))` def-env with a tuple-of-(list, recursive-sum) value works end-to-end.
- ✅ **B1b DEF-ENV TYPE-SHAPE DE-RISKED (2026-07-22 tick 15 probe): `Map(Int64, (List(Int64), Core))` — a Map
  whose value is a TUPLE of a `List(Int64)` and a recursive-sum `Core` — COMPILES + RUNS correctly** (probe:
  insert two entries, look up, destructure `Option.Some((params, body))` then match `body` on CN/CC → correct
  arithmetic, 2153-byte wasm → expected value). So B1b's `lower-def-env` return type + eval's `defs` env need NO
  fallback — the compound Map-of-(List, recursive-sum) tuple is fully supported. (Mirrors the pre-B1a `List(Core)`
  de-risk; the two together clear both novel type-shapes Slice B introduces.)
- The def bodies themselves must be lowerable. TODAY lower is demand-driven from a root; a recursive body never
  gets lowered standalone. NEW: `lower-def-env(tree) : Map(Int64, (List(Int64), Core))` — for every name in the
  def-table, lower its body ONCE to Core, keyed by name → (paramBinderIds, bodyCore). The body's own recursive
  self-calls lower to `CCall` (base case: when lowering a body we're already inside the def-env build, so a
  self-CCall just references the name — no recursion in the LOWERING because CCall doesn't descend into the body).
  This BREAKS the infinite-inline: CCall is a leaf w.r.t. lowering.

### B2 — infer/resolve stop declining recursion
- infer-db NApp arm (infer-db.cdz:105): the `call-is-recursive` branch returns TErr today. CHANGE: type a
  recursive call as its declared/inferred RESULT type. Monomorphic-i64 today → type the recursive call by
  typing the callee body ONCE under the param bindings with the self-call assumed to be the body's result type
  (a 1-step fixpoint; since everything is i64 the fixpoint is immediate — the recursive call is i64, args i64,
  result i64). Concretely: bind params to arg types, type the body treating a self-CCall as t-int-deferred/i64,
  take the body's type. Guard the arg fit (param-fit-ok) exactly as the non-recursive param arm does.
- resolve-db NApp arm (resolve-db.cdz:88): the `call-is-recursive` branch skips body resolution today. CHANGE:
  resolve the args in the caller scope (already done above the guard) AND resolve the callee body ONCE in its
  own param scope (lexical), keyed so lower can find CVar binders. The body is resolved standalone (like B1's
  def-env) — a self-call NApp inside it resolves to the def name (no descent), no infinite resolve.
- `call-is-recursive` STAYS as the discriminator (inline vs CCall); it is no longer a decline signal.

### B3 — eval runs recursion via a def-env (eval-db.cdz `eval-core`)
`eval-core` gains a second env: `defs: Map(Int64, (List(Int64), Core))` (name → (paramBinderIds, bodyCore))
built by `lower-def-env`. Add the arm:
```
| Core.CCall(name, args) =>
    (match Map.lookup(defs, name) with
      | Option.Some((params, body)) =>
          // eval each arg in the CURRENT env, bind positionally to the callee's param binder-ids in a FRESH
          // env (lexical: the body sees ONLY its params + defs, not the caller's locals), then eval the body.
          (match eval-args(args, env, defs) with
            | Option.Some(vals) => eval-core(body, bind-params(params, vals, Map.empty), defs)
            | Option.None(_) => Option.None(unit))
      | Option.None(_) => Option.None(unit))
```
Recursion terminates by the PROGRAM's base case (the `if (< n 1) ..`), exactly like the reference — NOT by a
fuel bound (a non-terminating source program non-terminates, which is correct; run-ml already tolerates the
32s-decline for the guarded case, and a real base case returns fast). `eval-core` threads `defs` unchanged
through every existing arm (CNum/CVar/CBin/CLet/CIf just pass it down). `eval-tree`/`run` build the def-env
from the tree first, then eval the root with it.

### B4 — emit (SEPARATE, later slice B') 
Today emit is nullary-main-only (single function, inlined body). A recursive callee needs a real wasm function
+ a `call` opcode + a loop/return. This is a bigger emit change (function table, locals for params, call
encoding) — DEFER to slice B'. Interpreter (B1–B3) makes run-ml green; emit-run + the W4 differential come in
B'. Sequence the MRs: B (interp) lands and closes the run-ml gap first; B' (emit) follows.

## ⚠️ SEQUENCING CORRECTION (2026-07-22 tick 21, verified from code — SUPERSEDES the "B1b before B2" order below for the LOWERING half)
The eval half of B3 (eval-core def-env + CCall dispatch, hand-built def-env @tests) is order-INDEPENDENT and
is DONE (committed `30788c508`, eval-db 59/0: fac/countdown/ev-od + empty-def-env-declines guard). BUT the
LOWERING half of B1b (`lower-def-env`) is COUPLED to B2 and CANNOT come first:
- infer marks a recursive NApp `TErr` (infer-db.cdz:105); resolve declines body resolution (resolve-db.cdz:88
  → returns colArg unchanged); `lower-node` bails to `None` on a `TErr`/missing fact (lower-db.cdz:67-72)
  BEFORE reaching lower-ok/lower-recursive-call. So a recursive body has NO ground tcol/rcol to lower through
  the real pipeline (this is exactly why B1a's @tests use a SYNTHETIC tcol + direct `lower-node`).
- Therefore `lower-def-env` needs B2 (infer+resolve stop declining, type/resolve each body once) FIRST — else
  it would duplicate B2's typing with a synthetic tcol, which is non-idiomatic (violates the operator's
  idiomatic-code directive). Idiomatic B2+B1b = a NAME-KEYED def-env pass that types/resolves/lowers each def
  body ONCE, where an inner self-call short-circuits as a name reference (no infinite loop). This is a
  MULTI-FILE coupled slice (infer + resolve + lower-db), NOT a one-tick unit.
- REVISED ORDER: B1a (done) → B3-eval (done, held) → **B2 (infer recursion arm — the REAL run-ml
  `(fac 5)` gate)** → B1b-lowering (`lower-def-env` + `run` wiring, on the B2 base) → B' (emit).
- ✅ **B2 is INFER-ONLY — resolve needs NO change (tick-24 PROBE, verified then reverted).** A probe hand-built
  fac's recursive body and called `resolve-node(body, {n→paramId}, empty)` directly: it TERMINATES and binds
  all three `n` uses to `RBound(paramId)`. The existing recursive-call arm (resolve-db.cdz:88) already resolves
  the call's ARGS in the current scope and does NOT descend the body — so when B1b's `lower-def-env` resolves a
  recursive body standalone in its param scope, the inner self-call hits this arm, resolves its args, and stops.
  No infinite descent, correct binding. So the resolve half of B2 is a NO-OP; only the infer arm changed (tick
  23, `1c77211d7`: recursive call → Int64 without body descent). This SAVES a slice.
- ✅ **B2-infer DONE (tick 23, WIP `1c77211d7`): infer-recursive-call types a recursive call as its i64 result
  without descending the body; infer-db 48/0, sread-eval-fns decline pins still hold.** Held from send behind
  the queued B3-eval MR.
- ✅ **B1b DESIGN DE-RISKED (tick-25 probe): `lower-def-env` MUST resolve+infer+lower each def body STANDALONE
  in its param scope — the whole-tree `resolve-tree`/`infer-tree` columns do NOT contain a recursive body's
  nodes.** A probe read the fac program via the real sread pipeline and measured `resolve-tree`/`infer-tree`
  fact counts over the root: resolve=0, infer=2 — i.e. the root walk sees ONLY main's body (the `(fac 5)` call
  node, now typed Int64 by B2-infer, + its literal arg `5`). fac's own ~10-node body is UNWALKED because the
  recursive-call arm (correctly) doesn't descend it. So `lower-def-env` cannot reuse the shared columns; for
  each def name in the def-table (enumerate via `Map.to-list` on `dt`) it must: build the body's param scope
  (`param-scope` over param-of/param2/3/4-of, same as the resolve NApp arm), `resolve-node(body, paramScope,
  empty)` (proven standalone-correct tick 24), infer the body standalone under the params bound to Int64, then
  `lower-node(body, rcol, tcol)` → the body Core (self-calls lower to CCall as leaves). Key → (paramBinderIds,
  bodyCore). This mirrors tick-24's standalone-resolve finding — both halves of a body's column-build are
  standalone, not shared-column reads.
- ✅ **B1b CONCRETE PREREQUISITE (tick-26): resolve-db must EXPORT `resolve-node` + `param-scope` (currently
  INTERNAL).** lower-db today imports only `resolve-tree`/`infer-tree` (whole-tree builders, lower-db.cdz:18-19).
  `lower-def-env` needs the per-body primitives: `param-scope(tree, paramId, empty)` to build the body scope +
  `resolve-node(body, scope, empty)` to resolve it standalone. Both live in resolve-db but aren't exported
  (tick-24's probe called them from WITHIN resolve-db). infer-db already exports `infer-node`; lower-db exports
  `lower-node`. PARAM TYPE SEEDING (verified via var-type.cdz:198): `var-type`'s NVar arm reads a param's type
  from `tcol` keyed by the param BINDER node-id — so to infer a body standalone, pre-seed tcol with each
  paramBinderId -> Int64 (`TIntW(true,64)`), THEN `infer-node(body, rcol, tcolSeeded)`; interior nodes then type
  ground (lower-node won't bail). So B1b = (1) tiny resolve-db export add [shippable standalone] + (2)
  `lower-def-env` in lower-db (enumerate dt via Map.to-list; per def: param-scope -> resolve-node -> seed params
  -> infer-node -> lower-node -> name->(paramBinderIds, bodyCore)) + (3) thread the def-env through db-lower/
  run-of-db into `eval-core-d(core, empty, defs)`.

## 🐛 B1b BLOCKER FOUND (tick-27): `if-type` rejects a MIXED deferred/concrete branch pair → must fix FIRST
Building `lower-def-env` (B1b part 2) surfaced a REAL infer-db gap. fac's body `if (n<1) then 1 else (n*(fac(n-1)))`
has then-branch `1` = DEFERRED int (`TIntW(_,0)`, an unconstrained literal) and else-branch = concrete `TIntW(true,64)`.
`if-type` (infer-db.cdz:496) decides the if's type via `type-eq(thenTy, elseTy)`, and `typed-to-ty` (infer-db.cdz:481)
maps `TIntW(s,w)` via `ty-fixed-int(s,w)` WITHOUT special-casing w=0 — so deferred→`ty-fixed-int(_,0)`, concrete→
`ty-fixed-int(_,64)`, `ty-eq` sees width 0≠64 → NOT equal → the if types **TErr** → `lower-node` bails → `lower-one-def`
declines fac (verified via standalone probe: body node = TErr code 3; call node + `*` node both ground). 
- WHY it's latent until now: the WHOLE-TREE query boundary grounds deferred (`ground-deferred` in `type-at`), and a
  NON-recursive `if`-body def (se-if-in-def-body `(if (< x 10) x 0)`) gets walked+grounded via the inlining path.
  fac's body is the FIRST body inferred STANDALONE (recursive → not walked), so it's the first to hit raw `if-type`.
  A two-literal if `(if (< 5 2) 10 20)` is fine (defer==defer via ty-eq-deferred-matches-deferred); only MIXED breaks.
- FIX (idiomatic, matches the existing deferred-grounding discipline): in `if-type`, GROUND both branch types before
  `type-eq` — a deferred (`TIntW(_,0)`) branch should unify with a concrete sibling's width (like `arith-result-type`
  grounds a deferred operand against a narrow sibling). Simplest: if one branch is deferred-int and the other is a
  concrete int, the if's type is the concrete one (ground the deferred to it); both deferred → deferred; else the
  existing `type-eq` rule. GATE: infer-db unit + a NEW pin `(if (< x 1) 1 (* x 2))`-shape standalone + re-run
  sread-eval `se-if`/`se-if-in-def-body` (must stay green — whole-tree path unaffected). This is a B1b PREREQUISITE
  (part 1.5), shippable as its own infer-db slice. THEN lower-def-env part 2 lowers fac's body clean.

## 🐛 B1b PART 3 BUG (tick-30): lower-def-env over a REAL multi-def tree traps — likely non-recursive body standalone
Wiring the def-env into run-of-db (`eval-core-d(core, empty, lower-def-env(tree))`) made the db-eval HAND-ARENA
`(fac 5)`→120 test PASS (mechanism works!), but `run-src` on a REAL sread tree `(do (def (fac n) ..) (def (main)
(fac 5)) (export main))` threw a RUNTIME error: `member access requires a record, found Type` + `Option has no
field Some/None`. cdz-check is clean (structural) — it's an eval-time trap.
- SUSPECT: `lower-def-env` lowers EVERY def-table body standalone, INCLUDING a non-recursive one like `main`
  whose body is `(fac 5)`. A non-recursive call inside a standalone body hits the INLINE path (lower-ok's NApp
  arm inlines the callee body), which needs the callee's param/arg wiring set up in the caller's column build —
  NOT present in lower-def-env's bare param-scope standalone build → malformed Core → eval trap.
- FIX (next tick): a `CCall` only ever targets a RECURSIVE callee, so the def-env only NEEDS recursive defs'
  bodies. Options: (a) in build-def-env, SKIP a def whose body isn't itself part of a recursion cycle (only add
  defs that are call-is-recursive targets — i.e. a def some CCall names); simplest = only add a def if
  `call-is-recursive(tree, name)` OR it's called-by a recursive body. (b) Cleaner: add ONLY defs that are
  actually CCall targets, discoverable by scanning lowered bodies for CCall names. Start with (a): `build-def-env`
  filters to `call-is-recursive(tree, nameId)` defs — fac IS recursive (added), main is NOT (skipped, stays
  inlined at its call site by the main lower-of-db). Re-gate: db-eval hand-arena + run-src fac/ev-od + the
  flipped sread-eval-fns pins (se-self/mutual-recursion-runs-slice-b1b) + a NON-recursive multi-def regression
  (se-two-distinct-helpers etc. must still run — they don't use the def-env).
- VERIFIED WORKING (keep): db-eval `de-recursive-fac-runs-via-def-env-slice-boneb` (hand-arena fac5→120, 9/0);
  lower-def-env on a lone recursive fac tree (lower-db 15/0); the eval mechanism (eval-core-d, landed B3).

## 🎯 ROOT-CAUSED (tick-34): a HOST monomorphization SCALING bug, NOT a defect in my B1b code — B1b UNBLOCKED
tick-33's file-swap bisection correctly fingered lower-db as the TRIGGER WEIGHT, but tick-34 proves the ERROR
is the HOST's (rcdzc), not my code's. DECISIVE bisection (source held fixed, varied component count/diversity):
- `cdz check sread-eval-fns` → CLEAN (0 err) — WELL-TYPED, no source error.
- `cdz test lower-db` → 0/0 — the lower-def-env chain compiles fine in its own component.
- 1 distinct run-src test + chain → PASS (value correct, v==6). 8 IDENTICAL run-src + chain → 8/0. 6 DISTINCT
  run-src programs + chain → 6/0.
- **36 distinct run-src tests (the real file) + chain → ~103 UNLOCATED CDZ0201/0203.** 36 distinct, baseline
  lower-db (trunk) → 36/0.
So the errors emerge ONLY from (heavier per-component monomorphization) × (many DISTINCT whole-pipeline run-src
components in one file) — a host component-build SCALING limit, misreported as `member access requires a record,
found Type` / `Option has no field Some/None` with NO source location. The chain is even DEAD CODE in the run-src
closure at this revision (p3 reverted) — mere module presence tips it. Anonymous-vs-named `Tuple` swap AND
`Map.to-list` removal both changed nothing → aggregate module weight, not one construct.
**CORRECTS tick-33's "B1b-p2/p2.5 MUST NOT SEND until fixed":** there is nothing wrong with lower-db to fix.
The blocker is the host limit + the 36-test FILE SIZE. FILED (operator SURFACE directive):
`queue/mlrepro-host-monomorph-scaling-spurious-cdz0201-under-many-run-src-components.md` + issue → corpus-bugfix
(asks: give build-stage diagnostics a location; raise per-file component-build cost).
**B1b UNBLOCKED PATH (no host-fix needed):** after B2-infer/B1b-p1 land + clean sync → send p1.5/p2/p2.5 → redo
p3 wiring → **SPLIT sread-eval-fns into 2 smaller files** (the same pattern that split it out of sread-eval for
the 360s timeout; see companion `vcml-design-conformance-db-file-split-throughput.md`) so each stays under the
~6–36 component threshold → run-src fac→120. db-eval 9/0 already proves the eval mechanism end-to-end.

## Slice/MR breakdown (each gated green + reference-checked, folded — no broken intermediate)
1. **B1a** — add `CCall` to Core + `lower-def-env` + lower emits CCall for recursive calls (lower-db unit
   @tests: a recursive NApp lowers to `CCall(name, [args])`; non-recursive still inlines). No eval yet →
   recursive still declines at run (eval has no CCall arm → None), but the lowering is pinned.
2. **B1b/B3** — eval-db CCall arm + def-env threading (eval-db unit @tests: hand-built `CCall` over a def-env
   runs fac/countdown; base-case terminates). run-ml `(fac 5)`→120 goes GREEN.
3. **B2** — infer/resolve recursion arms (so the SOURCE pipeline, not just hand-arena, types+resolves a
   recursive def). sread-eval-fns @tests: `se-self-recursion-RUNS` (was -declines-not-hangs), `se-mutual-
   recursion-runs`. Update the two `-declines-not-hangs` tests to `-runs` (they currently pin the decline).
4. **B'** (later) — emit a real wasm function + call for a recursive callee; run-emitted + W4 differential.

## Test migration (IMPORTANT — the forward-ref reject lesson)
The existing `se-self-recursion-declines-not-hangs` / `se-mutual-recursion-declines-not-hangs` @tests PIN the
CURRENT decline. When recursion RUNS they must FLIP to assert the value (fac 5 → 120). Per the 2026-07-21
reject lesson: after ANY of these arms change, self-run `cdz test` on eval-db, lower-db, infer-db, resolve-db
AND sread-eval-fns BEFORE sending — the gate --check corpus does NOT cover these eval shapes.

## REFERENCE-VALIDATION (rcdzc, 2026-07-22 — read breaker worktree's src/core.rs + src/lower.rs)
Cross-checked the design against the Rust reference compiler (the operator's "use rcdzc as guide" directive).
Every core decision is CONFIRMED by the reference; a few nuances surfaced:
- ✅ **`Core::Call { callee: usize, args: Vec<StructId> }`** (core.rs:1061) — the reference's recursion node is
  EXACTLY my `CCall(calleeName, args)` shape (callee id + arg list). Design confirmed.
- ✅ **Recursion is a STATIC CODE REFERENCE, never a heap cycle** — core.rs:1061 cites the spec: "A recursive
  definition MUST refer to itself through a static reference to code rather than through a value that points
  back into the heap" (memory-and-resource-model.md §The Value Heap Is Acyclic). My def-env keyed by NAME (name
  → (params, bodyCore)) honors this — the self-call is a name lookup, not a heap pointer. Sound.
- ✅ **Only a NAMED top-level def can emit a Call; a computed/anonymous recursive head DECLINES**
  (lower.rs:11292 `callee_def_index` None → `Reject::decline`). My plan matches: only a def-table name emits
  `CCall`; unknown name → decline (def-body-of None). Keep this.
- ✅ **Always-INLINE is the default; emit-once only for LARGE multiply-called runtime-arg helpers** (lower.rs
  `should_emit_once_by_cost`: `INLINE_COST_THRESHOLD=40` nodes AND `INLINE_MIN_CALLERS=2`). The operator
  "kept always-inline as the observable default during the compiler-port". So my "inline all non-recursive,
  CCall ONLY for recursive" is the reference's conservative default — no cost heuristic needed for Slice B.
- ⚠️ **NUANCE for emit (B'), not the interpreter (B1–B3): the reference has a SEPARATE `Core::Param { binder }`
  node** (core.rs:1064) for a param inside a STANDALONE-lowered function body — the backend maps it to a
  `local.get` of the param's wasm slot, DISTINCT from an inlined var. My interpreter plan reuses `CVar(binderId)`
  with a per-call env (fine for eval — a name lookup either way). But when I do B' (emit a real wasm function),
  the recursive callee's params must become wasm LOCALS (local.get), so B' likely needs a `CParam` distinction
  (or emit maps CVar-bound-to-a-param → local.get by checking the def-env param list). Note for B'; does NOT
  affect B1–B3.
- ⚠️ **Reduction/recursion BUDGET decline is a CODED reject** (`Code::RecursionBound`, lower.rs:11282) — the
  reference stops+reports on a non-normalizing term, never hangs. My interpreter defaults to faithful (no fuel);
  if the test harness needs a safety net, code it as a coded decline (not a bare None) mirroring this.

## Risk / open questions
- Mutual recursion (ev/od): `call-is-recursive` already walks transitively (subtree-calls-callee), so it
  flags both — both emit CCall, both in the def-env. Should work; add an ev/od @test.
- Non-termination: a genuinely non-terminating recursive source (no base case) will loop in eval (correct, but
  run-ml would hang). Keep such programs OUT of the @test suite (the guarded-decline tests already avoid them);
  if a corpus program non-terminates that's a source bug, not ours. Consider a LARGE step-fuel in eval-core ONLY
  as a test-safety net if the harness needs it — but default is faithful (no fuel), matching the reference.
- Param binder-ids: the def-env keys params by their NVar binder node-ids (same as CLet today), so the body's
  CVar(binder) lookups hit the fresh per-call env. Verify the param binder-id is stable across calls (it is —
  it's the def's param node-id, fixed at parse).
