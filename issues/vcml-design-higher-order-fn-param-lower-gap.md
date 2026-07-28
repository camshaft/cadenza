# Design/scoping: HIGHER-ORDER functions decline — lower-db has no var-headed-application arm

**Found:** 2026-07-22 (tick 187), v-compiler-ml, generics-increment scoping probe (host idle). NOT a bug to
report to corpus-bugfix — this is compiler-ml's own expected frontier (the next increment), precisely localized.

## The frontier (run-ml probe, all on trunk@77f5e96f8)
GREEN (works today):
- polymorphic identity single + double use: `(def (id x) x) (main) (+ (id 1) (id 2))` → 3
- id at TWO instantiations (Bool AND Int64): `(if (id (< 1 2)) (id 100) (id 200))` → 100  (HM generalize/instantiate LIVE)
- `(def (const a b) a) (+ (const 5 (< 1 2)) (const 10 20))` → 15  (rank-1 poly, 2 instantiations of 2nd arg)
- `(def (fst a b) a) (fst 10 20)` → 10

DECLINES (the gap):
- `(def (apply f x) (f x)) (def (inc n) (+ n 1)) (main) (apply inc 41)`   → declined (expect 42)
- `(def (twice f x) (f (f x))) ... (twice inc 40)`                        → declined (expect 42)

## Root cause — PRECISELY localized (not inference; it's LOWER)
- `cdz check` PASSES (exit 0, no error) on `(def (apply f x) (f x)) ...` → inference ALREADY types the arrow
  (function-typed param). The HM scaffolding in type-scheme.cdz + unify handles `f : a -> b`.
- The decline is at LOWER: `lower-db.cdz` NApp arm (line ~116-134) resolves a call's callee via
  `def-body-of(tree, calleeId)` — it assumes the call HEAD is a known DEF NAME (inline the body / emit a CCall
  by name). When the head is a PARAMETER (`f` in `apply f x = f x`), `def-body-of` returns `None` → line 134
  `Option.None(unit)` → clean decline (no wrong value — SAFE).
- So: **lower-db has NO arm for a var-headed application** — an application whose callee is a bound
  variable/parameter (a first-class function value) rather than a top-level def name.

## Why it matters (the generics/HO increment)
Higher-order functions (map/fold/filter-style, `apply`, `twice`, callbacks) are THE payoff of the polymorphism
the front-end already supports. Inference is ready; the missing piece is representing + lowering + evaluating a
FUNCTION VALUE (closure/fn-ref) passed as an argument and called.

## Smallest first slice (proposed — for the idle-host build tick)
1. **Represent a fn value in Core.** A new Core node for a function reference (a def's funcidx/name as a
   first-class value) — the arg `inc` in `(apply inc 41)` lowers to a fn-ref Core value, not an inline.
2. **Lower a var-headed NApp.** When `def-body-of(calleeId)` misses BUT the callee resolves to a BOUND PARAM
   holding a fn value → lower to an indirect-call Core node (callee = the bound var's fn value, args lowered).
3. **eval-core** an indirect call: look up the fn value in the env, apply it (bind its params to the args, eval
   its body) — mirrors the existing CLet-inline for a known callee but dispatched on a runtime fn value.
4. Emit (B''): a var-headed call → wasm `call_indirect` (or, for the monomorphic-def-ref case, resolve the
   concrete funcidx at lower time and emit a direct `call`). START with eval-only (run-ml green) like Slice B did.

Mirror rcdzc's closure/indirect-call lowering as the guide. Keep the self-only-def-env recursion model intact.
Coordinate with v-inference (they OWN infer/unify) if the arrow-type inference needs any widening for the
fn-value representation — but check ALREADY passes, so likely lower/eval-only to start.

## CONCRETE eval representation (tick-188 shovel-ready read of eval-db.cdz + lower-db.cdz)
The eval model is SIMPLER than a Value-ADT closure — exploit it:
- `eval-core-d(c, env, defs)`: `env : Map(Int64→Int64)` (binderId→VALUE, values are Int64), `defs : Map(name →
  (List(Int64) params, Core body))`. `CCall(name, args)` already dispatches by NAME through `defs` (L89-101):
  eval-args in caller env → bind-params positionally into a FRESH env → eval body.
- **A function VALUE = its def-NAME as an Int64.** No closure ADT needed for the top-level-def case (no free-var
  capture — defs are closed). So:
  - Passing `inc` as an arg (`(apply inc 41)`): the arg `inc` (a bare def-name used as a value) lowers to a Core
    value carrying inc's name-id → at eval it's just that Int64 name in `env` bound to `apply`'s param `f`.
  - Calling a param `(f x)` where `f` is a fn-value param: lower to a new Core node `CCallVar(fBinderId, args)`;
    eval looks up `env[fBinderId]` = the callee NAME, then does the SAME defs-dispatch as CCall (factor the
    CCall body into a shared `apply-def(name, argVals, defs)` helper and call it from both arms).
- Core additions: (a) `CFnRef(Int64)` — a def-name as a first-class value (lowers from a bare def-name in value
  position); eval returns `Some(name)`. (b) `CCallVar(Int64, List(Core))` — apply the fn-value in binder `f` to
  args; eval = `apply-def(env[f], eval-args, defs)`. (Could unify (b) into a `CApp(Core callee, args)` if a
  computed/returned fn is wanted later; start with the CVar-callee case = the probe's `apply`/`twice`.)
- Lower: in the NApp arm (lower-db L116-134), when `def-body-of(calleeId)` MISSES, check if calleeId resolves to
  a fn-typed PARAM binder → emit `CCallVar(binderId, loweredArgs)`. A bare def-name in ARG position → `CFnRef`.
- Emit (B''): defer — CFnRef→i64 const of a funcidx, CCallVar→`call_indirect` via a table; start EVAL-ONLY
  (run-ml green), exactly like Slice B did before emit(B').

## Watch-outs
- `defs` only holds SELF-RECURSIVE defs today (lower-def-env, PR#785 self-only). A passed fn like `inc` is NON-
  recursive → it must ALSO be in the dispatch table for CCallVar to find it. So the fn-value slice needs a def
  table keyed by EVERY callable def-name (not just recursive ones) — extend lower-def-env (or a sibling
  fn-table) to include non-recursive defs referenced as values. Verify this doesn't perturb the CCall recursion
  path (recursive dispatch stays; we just ADD the non-recursive entries the fn-value path needs).
- Inference already types it (`cdz check` passes) — so likely NO v-inference change; coordinate only if the
  fn-value Core node needs a type-fact the lower reads that infer doesn't currently expose.

## BLAST RADIUS (tick-189/190 read — where adding CFnRef + CCallVar to `type Core` ripples)
Only TWO exhaustive (no-wildcard) Core matches — these MUST get new arms:
- `eval-db.cdz eval-core-d` (ends at the CCall arm ~L102, no trailing `_`): ADD `CFnRef(name) => Some(name)`
  and `CCallVar(fBinder, args) => apply-def(env[fBinder], eval-args…, defs)` (factor `apply-def` out of the
  existing CCall arm L94-102 — same `Map.lookup(defs,name)`→eval-args→bind-params→eval body).
- `emit-db.cdz can-emit` (6 arms, no wildcard, L179): ADD `CFnRef(_) => false` and `CCallVar(_,_) => false`
  (out of emit subset — eval-only slice; emit(B'') later). This is what makes emit DECLINE the HO program cleanly.
Everything else auto-declines via `| _ =>` wildcards (SAFE, no edit needed): emit-db `collect-binders` (L205),
`emit-instrs-d` (L253), emit-rec-db `can-emit-d`; lower-db's small matches (is-cnum/is-cvar L322/325) are
wildcard too. So the WRITE surface = `type Core` (lower-db L23) + eval-core-d + can-emit + the lower NApp
miss-arm + def-table extension. Contained; no cross-file exhaustiveness cascade.

## SLICE PLAN (v-inference greenlit tick-195; they own rcdzc reference, I build in compiler-ml w/ their review)
- **HO-1 (arrow type) — ✅ DONE + MR'd `f43dd0af2` (tick-195, trunk@f68c33940):** Ty.TyFn(List(Ty),Ty) N-ary +
  unify-ty arrow arm (same-arity + pairwise param unify + result unify, rcdzc unify.rs:209 generalized) + ty-eq
  structural arrow arm + TyFn→Typed=TErr bridge stopgap. Gate ty 15/0, unify-ty 13/0, ty-bridge 6/0, infer-db 53/0.
- **HO-2a (resolve RDef) — ✅ DONE + MR'd `11d997588` (tick-197, stacked on HO-1):** Resolved.RDef(Int64) +
  resolve-db NVar-miss→def-body-of fallthrough (scope-then-def-table). RDef inert downstream (RBound-only
  consumers decline via wildcards). Gate resolve-db 12/0, db-resolve 4/0, + infer/lower/eval regression-clean,
  fac→120/inc→42 unchanged. v-inference APPROVED HO-1 (review note tick-197).
- **HO-2b-i (Typed.TFn + real bridge) — ✅ DONE + MR'd `5d1a178c3` (tick-198, 3-deep stack CAP):** Typed.TFn +
  typed-to-ty/ty-to-typed real recursive TFn↔TyFn arms (replaced HO-1 TErr stopgap). Gate ty-bridge 8/0, infer-db
  53/0, whole-pipeline unchanged. Inert (nothing builds a TFn yet).
- **HO-2b-ii (infer NApp arm types arrows) — NEXT (after stack drains — at cap now), sites pinned tick-199:**
  TWO edits in infer-db (both need `TFn`-building from a def's params+body-type):
  - (1) NVar arm (L73-74): currently RDef → TErr (via `| _ =>`). ADD: an RDef(defName) node → build the def's
    ARROW `Typed.TFn([paramTypes…], bodyResultType)`. Param types via `param-type-of(tree, paramId)` (declared
    narrow) or default Int64 for each of param-of/param2/3/4-of; result = infer the def body's type. This types
    a bare def-name-as-value (`inc` in `(apply inc 41)`).
  - (2) NApp arm (L112-113): `def-body-of(calleeId)` MISS currently → TErr. ADD: if calleeId is a fn-typed PARAM
    binder (resolve says RBound to a param whose var-type is a TFn), type the call as that TFn's RESULT (after
    checking arg types unify with the TFn's param types). This types `(f x)` where f is a fn param.
  - Helper: `def-arrow-type(tree, defName, tcol)` = build TFn from param-of/2/3/4 (param-type-of or Int64) +
    infer-node(body). Reuse in both sites. v-inference REVIEWS the NApp fn-param arm + the arrow construction.
  - After HO-2b-ii: `(apply inc 41)` + `(twice inc 40)` TYPE (cdz check passes with real types, not decline).
    HO-3 lower/eval (stashed `e62b3584f`) then makes them RUN → gate `(apply inc 41)`→42.
  - **SPLIT (tick-200): HO-2b-ii-A (clean) vs HO-2b-ii-B (design-forked, ASKED v-inference):**
    - **HO-2b-ii-A** = NVar arm types an RDef-value → def's arrow TFn. ✅ BUILT + committed `c951f7058` (tick-202,
      4th stacked, HELD-not-sent at cap). def-arrow-type resolves the body in the def PARAM SCOPE (crucial — else
      body param vars have no RBound → result TErr). Gate infer-db 55/0 (2 arrow tests), regression clean, fac→120/
      inc→42 unchanged, HO prog still declines. SEND when a stack slot frees.
    - **HO-2b-ii-B** = type `(f x)` with f a fn-PARAM. v-inference STEERED **option (i)** (tick-206): rcdzc's App
      head is a resolved NODE (StructId) typed like any var-ref (`Resolved::Apply{head:StructId}` resolved.rs:1282,
      apply_type infer.rs:4595); a param-head resolves to Param w/ its arrow type, NO special-case. compiler-ml's
      divergence: `NApp(calleeName: Int64, argId)` stores the callee as a NAME-ID field (parse-db:51), not a node,
      resolved ONLY via def-body-of.
      **CONCRETE option-(i) plan (tick-208, since the callee is a name-id w/ no node to key by):** in resolve-db's
      NApp arm, ALSO resolve the callee-name against the CALLER env (scope-first: param+let→RBound, else def-table
      →RDef) and STORE that fact KEYED BY THE NApp NODE-ID `id` (the call site) — a "callee-resolution" fact. Then
      infer's NApp arm, BEFORE the def-body-of path, reads `Map.lookup(rcol, id)`: if RBound(paramBinder) whose
      var-type is a TFn → type the call as that TFn's RESULT (checking arg types vs the TFn param types); if RDef
      → def-arrow path (HO-2b-ii-A already types a def-ref, reuse). If no callee-fact (ordinary global-def call) →
      the EXISTING def-body-of path unchanged. Keying by the NApp id is safe (the NApp node's own resolve slot is
      otherwise unused — it carries no var). This same callee-fact feeds HO-3's lower (param-callee→CFnRefVar).
      Build stacked on HO-2b-ii-A; v-inference reviews the resolve callee-fact + infer NApp fn-param arm.
      ✅ DONE + MR'd `cae7994ef` (tick-211): resolve `colC` callee-fact keyed by NApp id + infer-fn-param-call.
      `(f x)` w/ fn-param TYPES (infer-db 57/0, 2 new tests). Stacked on HO-2b-ii-A. HO-3 makes it RUN.
- **HO-3 (lower + eval) — 🚧 BUILT but BLOCKED by a host build-scale bug (tick-213):** wired it fully — lower
  NVar RDef→CFnRef + NApp-miss→CFnRefVar (via the callee-fact) + eval arms (stash). BUT: adding the eval-db HO
  arms tips the `cdz run-ml` WHOLE-PIPELINE component build → declines EVERYTHING (bare 42→declined), while
  cdz check clean + eval-db's OWN test-component fine 59/0. BISECTED: eval-db's arms are the tipper, only in the
  aggregate run-ml closure = codegen-at-scale (func[58] family?). FILED `queue/mlrepro-eval-db-ho-arms-tip-run-ml-
  pipeline-component-build.md` + reported to v-wasm-opt (issue). ⚠ the def-table extension (include non-rec defs
  for apply-def-by-name to find `inc`) is ALSO needed + is tick-30-bug-territory (naive blanket-add broke
  fac/inc/add) — NEEDS a targeted policy (only fn-value-referenced defs, or a separate fn-table), design w/
  v-inference. HO-3 HELD pending: (1) v-wasm-opt host build fix OR an eval-db split mitigation, (2) the safe
  def-table policy. HO TYPING (HO-1..2b-ii-B) is DONE/landing regardless — only RUNNING HO programs is blocked.
- **HO-2c (arg↔param arity+type check) — ✅ DONE + MR'd `d289af7c5` (tick-216, stacked on HO-2b-ii-B):**
  args-fit-params (arity + pairwise unify-ty, typed-to-ty-DEFER for the arg). infer-db 59/0 (3 new tests),
  whole-pipeline unchanged. Closes the HO-typing follow-up. 🪤 the DEFER matters: a bare-literal arg is
  TIntW(_,0); plain typed-to-ty maps width-0→bogus fixed-Ty that unifies w/ nothing → the fitting case wrongly
  fails. Caught by the existing arrow-result test.
- **HO-2c (original scoping tick-214, v-inference's approved follow-up, NOT run-ml-blocked):**
  infer-db `infer-fn-param-call` (L167) currently does `TFn(_, rty) => rty` — IGNORES param types. HO-2c: match
  `TFn(ptys, rty)`, and before returning `rty` UNIFY each arg's type (at argId/arg2-of/arg3-of/arg4-of, types in
  ta4) vs the corresponding `ptys` element (arity: len(args)==len(ptys); type: unify-ty via bridge, mismatch →
  TErr/CDZ0203). All fit → rty; else TErr. Self-contained infer-db slice, stacks on HO-2b-ii-B, gates via
  `cdz test infer-db` (builds fine — NOT the run-ml whole-pipeline blocker). Independent of HO-3 (host-blocked) —
  buildable as soon as host calms.
- **HO-2 (infer + resolve) — original scoping tick-196 (build on LANDED HO-1):** exact sites pinned:
  - (A) resolve-db `resolve-node` NVar arm (L42-45): `Map.lookup(env, nm)` MISS currently → `RPoison`. Change:
    fall through to `def-body-of(tree, nm)` — if it's a def-name, classify as a def-ref VALUE. ⚠ compiler-ml's
    `Resolved` = ONLY `RBound(Int64) | RPoison` (NOT rcdzc's richer Ref/Param/Lambda) → HO-2 MUST ADD a new
    variant `RDef(Int64)` (the def-name). RIPPLE: 8 files match Resolved (db, db-resolve, eval-db, resolve-db,
    parse-db, infer-db, lower-db, sread) — most via wildcards, but `cdz check` will flag the exhaustive ones
    (audit each; RDef→ the fn-value path in lower/eval, else poison in int-only contexts). Also: a CALL whose
    callee `f` is a param — the NApp callee is a name-id resolved via def-body-of (parse-db), NOT resolve-node;
    so "f is a fn-param" is detected at LOWER (def-body-of miss + f in param scope), not resolve. Simplest: (A)
    only adds RDef for a bare def-name-as-VALUE (the `inc` arg); the param-callee `(f x)` detection stays in
    lower's NApp miss-arm (HO-3), checking if calleeId is a param binder.
  - (B) infer-db: `Typed` (= `TIntW|TBool|TErr`) MUST gain a fn variant `TFn(List(Typed), Typed)` — ripples to
    every infer-db Typed match (arith-result, if-join, etc. — mostly wildcard/TErr-safe). infer NApp arm (L95):
    a def-name-as-value → the def's arrow Typed; `(f x)` with f a fn-param → the arrow's result. Bridge TyFn↔TFn
    in ty-bridge (replace the HO-1 TErr stopgap). v-inference REVIEWS (their (A)+(B) greenlight).
  - GATE: infer-db + resolve-db + a run-ml-adjacent check. Big slice — may split HO-2a (RDef resolve) / HO-2b
    (Typed.TFn infer). Build on LANDED HO-1 (HO-2's TyFn use depends on it).
- **HO-3 (lower + eval) — my stashed WIP `e62b3584f`:** Core CFnRef/CFnRefVar + eval apply-def-by-name + lower
  NApp miss→CFnRefVar / bare-name→CFnRef + def-table extension to non-recursive defs. Gate run-ml `(apply inc 41)`→42.

## PROGRESS (tick-191): FOUNDATION sub-slice WRITTEN + cdz-check-clean (UNCOMMITTED working-tree WIP)
DONE (all 3 files `cdz check` exit 0):
- lower-db.cdz `type Core`: added `CFnRef(Int64)` (def-name as fn value) + `CFnRefVar(Int64, List(Core))` (apply
  a fn-value binder).
- eval-db.cdz: factored `apply-def-by-name(name, args, env, defs)` out of the CCall arm; CCall now calls it;
  added `CFnRef(name)=>Some(name)` and `CFnRefVar(fBinder,args)=>apply-def-by-name(env[fBinder], …)` arms.
- emit-db.cdz `can-emit`: added `CFnRef(_)=>false` + `CFnRefVar(_,_)=>false` (eval-only). emit-rec-db can-emit-d
  auto-declines via its `| _ =>false`. emit-instrs-d/collect-binders auto-decline via wildcards.
REMAINING (the hard part — do next, needs careful tree-representation tracing + full cdz test gate):
- lower-db NApp arm (L127-175): when `def-body-of(calleeId)` MISSES (L145) AND calleeId resolves to a fn-typed
  PARAM binder → emit `CFnRefVar(binderId, loweredArgs)` instead of declining. MUST first trace: how does sread
  represent `(f x)` with f a param (NApp calleeId = the param's resolved binder node?) and a BARE `inc` in arg
  position (NVar? nullary NApp?) — the CFnRef construction site depends on this. DO NOT guess; read sread/resolve.
- def-table extension: `defs`/lower-def-env is SELF-RECURSIVE-ONLY (PR#785). A passed non-recursive fn (`inc`)
  must ALSO be in `defs` for apply-def-by-name to find it → extend lower-def-env (or a sibling fn-table merged
  into `defs` before eval) to include EVERY callable def referenced as a value. Verify no perturbation of the
  CCall recursion path.
- GATE: `cdz test` eval-db + a run-ml `(apply inc 41)`→42 + `(twice inc 40)`→42 pin (add to sread-eval-fns or a
  sibling). Was NOT gate-able tick-191 (host loadavg 51→10 unstable, peer batch).
NOTE: foundation edits are UNCOMMITTED working-tree WIP (dead until lower wiring, so not shippable alone). Fully
recreatable from this note. Do NOT `git reset --hard` / blind-stash (shared stash) while these are live.

## Status (superseded by PROGRESS above)
Scoped + eval-representation + blast-radius all nailed. Foundation written+check-clean tick-191. Next: the lower
wiring + def-table + gate. Original plan retained below:
CFnRef + CCallVar (Core), the lower NApp miss→CCallVar arm + bare-name→CFnRef, factor apply-def shared helper,
extend the def-table to non-recursive defs, gate a run-ml `(apply inc 41)`→42 + `(twice inc 40)`→42 pin
(EVAL-ONLY). Blocked on nothing (emit(B') is v-cdz-tooling's flip; this is the parallel next increment).
