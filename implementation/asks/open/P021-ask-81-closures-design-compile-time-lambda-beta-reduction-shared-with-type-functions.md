# 81. 🧭 (rcdzc design + implementation handoff) Closures: a lambda is a compile-time value the `eval` fold β-reduces — the SAME tier that will do type-functions / monomorphization

**What.** The plan for adding `(fn (x) body)` lambdas + closures to the native rewrite `rcdzc`, written
ahead of implementation so the work is built on the right foundation. This is a design/handoff ask (like
ask-75 for inference): it states the target, the IR changes, the pass-by-pass edits, the accept/decline
boundary, the corpus it unblocks, and the subtleties an implementer must get right. It is scoped into two
increments; **Increment A is the concrete handoff**, Increment B is architected but deferred/declined.

The trigger is the operator's steer to "get ahead of designing closures" and to align them with the
**first-class-types / type-functions** direction (task #150, the NEXT rewrite increment). The alignment is
not incidental — as §"The type-functions connection" shows, **closures and type functions are the same
mechanism** (a lambda + compile-time β-reduction), so building the lambda reduction tier once serves both.

---

## The core insight

rcdzc already has **no runtime function values**. A `FuncRef` (a module function as a value), a `Ctor`
(a sum constructor value), and an `Intrinsic` (a built-in-op value) are all **transient compile-time
values**: the `eval` fold (`fold.rs`) reduces every *application* of one to a concrete construct
(`Apply(FuncRef,args)`→`Call`, `Apply(Ctor,arg)`→`Mir::Sum`, `Apply(Intrinsic,args)`→`emit_intrinsic`),
and a *bare survivor* that reaches `select` **declines** ("a function value cannot yet cross to run time").

**A lambda is exactly one more transient compile-time value in that family, and its application is
β-reduction — which the fold already IS.** So closures are not a new subsystem; they are:

1. one new IR leaf (`Lambda`) that flows Hir→Typed→Mir like `FuncRef`/`Ctor`/`Intrinsic`;
2. one new fold rule: `Apply(Lambda, args)` → substitute params with args into the body, then fold
   (β-reduction — the identical reduction that inlines a `Call` and that will monomorphize a generic); and
3. one `select` decline for a `Lambda` that *survives* the fold (a genuine runtime closure — Increment B).

Everything the core function/closure corpus (`spec/semantics/09-functions.sexp`) requires reduces at
compile time, so **Increment A (compile-time lambdas) closes the whole core `Functions` witness** without
any runtime-closure machinery.

---

## Two increments

### Increment A — compile-time lambdas + β-reduction (THE HANDOFF)

Lambdas are transient compile-time values; the fold β-reduces every application. A lambda that reduces
away (applied in place, applied through an inlined non-recursive HOF, selected by a const `if`) compiles;
a lambda that **survives** the fold to `select` declines. This closes the core `09-functions.sexp` cases:

| corpus case (09-functions.sexp)                                   | reduces via                                              | A? |
|-------------------------------------------------------------------|----------------------------------------------------------|----|
| `((fn (x) (+ x 1)) 5)` → 6                                        | `Apply(Lambda,[5])` β-reduce → `(+ 5 1)` → 6             | ✅ |
| `(let ((inc (fn (x) (+ x 1)))) (inc 10))` → 11                    | `let` inlines the transient Lambda, then β-reduce         | ✅ |
| closure captures `y=3` const → 7                                  | inner `let y=3` folds to const, β-reduce                  | ✅ |
| `apply-twice` (let-bound HOF) → 7                                 | inline `apply-twice`, β-reduce the two `(f …)`            | ✅ |
| **named HOF `(def (ap g v) (g v))` receives a lambda** → 14        | force-inline `ap` (fn-typed arg), β-reduce `(g v)`        | ✅ |
| `adder` returns a closure, `((adder 10) 5)` → 15                  | β-reduce `(adder 10)`→Lambda, then `Apply(Lambda,[5])`    | ✅ |
| record builder `((fn (x) (record (v x))) 7)` field-projected → 7  | β-reduce, then record projection (already works)          | ✅ |
| **curried application completed to a direct call** `((add 3) 4)` → 7 | spine-collapse `Apply(Apply(FuncRef,[3]),[4])`→`Call{add,[3,4]}` | ✅ |
| lambda stored in a **tuple** element, extracted, called → 6       | needs product-projection to see the transient element     | ✅* |

\* the tuple/list-storing case needs the compile-time-product projection generalized (see
§"Accept / decline boundary"); it is A-with-a-small-extension, and declines cleanly until then
(reject-don't-miscompile — the corpus explicitly permits declining it).

Note a **runtime capture reduces fine in Increment A** as long as the lambda is β-reduced *in place*:
`(def (f n) ((fn (x) (+ x n)) 5))` β-reduces to `(+ 5 n)` where `n` is `f`'s runtime parameter — a valid
`Arith` node. Increment A only declines a lambda that **cannot be reduced at all** (it escapes to run time).

**Currying that const-folds back into a regular call IS supported (Increment A).** Functions are
single-arity in the spec, so `(f a b)` ≡ `((f a) b)`; a compiler authored in Cadenza writes curried
applications and partial applications constantly. When a partial application is **completed to full
arity at compile time**, the fold must **reassemble the direct `Call`**, NOT decline it:
- `((add 3) 4)` where `(def (add x y) (+ x y))` → the fold collapses the application spine
  `Apply(Apply(FuncRef(add),[3]),[4])` into `Apply(FuncRef(add),[3,4])`, reaches full arity, and emits
  `Call{add,[3,4]}` (which then `try_inline`s to `7` for const args, or stays a direct runtime call). This
  is the "curried and const-folded into a regular function call" case — an ordinary compile-time reduction,
  the same tier that will uncurry a type-function application (§"The type-functions connection").
- `(let ((add3 (add 3))) (add3 4))` → `add3` binds the transient partial application (a `FuncRef` applied
  to `[3]`); the `let` inlines it, the spine collapses, → `Call{add,[3,4]}`.
- A partial application that **never reaches full arity at compile time** (it escapes — passed to a
  recursive HOF, stored in runtime data, selected by a runtime branch) is a surviving under-arity value →
  DECLINE (Increment B: it needs a runtime closure). This is the ONLY partial-application decline.

The mechanism is spine collapsing over the fold's existing `arities` table + β-reduction — no η-expansion,
no new IR. See `fold.rs` change #2 and §"Accept / decline boundary".

### Increment B — runtime closures (materialized environment + `call_indirect`) — DEFERRED

The genuinely-runtime case: a lambda that survives the fold because it escapes — stored in a runtime data
structure, selected by a runtime `if`/`match` and then applied, or applied through a **recursive** HOF the
fold cannot inline. This needs, in the target:

- a **wasm function table + `call_indirect`** (new `Lir` ops + a table/element section in `serialize` /
  `component.rs` — neither exists today; `grep` confirms no `call_indirect`/table anywhere in rcdzc);
- **closure conversion**: lift each escaping lambda to a top-level function taking an explicit environment
  parameter, and materialize a heap **environment record** (an `arr`, the existing product rep) holding the
  captured runtime values; the closure value is `(code-index, env-handle)`;
- **`Ty::Fn` gets a `core_valtype` of `I32`** (a closure handle) — today it is `None` (the "no runtime
  function value" invariant). This is the single type-model change B makes and A must NOT.

B is a large lift and off the self-host critical path (the compiler-in-Cadenza's own HOFs are
monomorphic/inlinable). **Increment A declines every B case with an honest message**, never miscompiles.
Design B now only to the extent of *not painting A into a corner* — which the transient-value model does.

---

## Why this matches the existing architecture

This is the FuncRef/Ctor/Intrinsic pattern applied once more (see `ir.rs` doc-comments on `Mir::FuncRef`
/`Mir::Ctor`/`Mir::Intrinsic` — "transient … present only between `lower` and `eval` … a survivor
declines"). It also realizes the locked architecture decision
([[rcdzc-native-rewrite-phase0-landed]]): *"monomorphization = compile-time β-reduction = the SAME `eval`
that folds constants … NO bespoke Λ/@ construct in the IR; a generic instantiation is just an Apply node
the `eval` pass reduces."* A lambda is that Apply's callee. Building β-reduction for value lambdas builds
it for type lambdas.

---

## IR changes (`ir.rs`, `ty.rs`)

Nanopass discipline: a new IR variant means every exhaustive `match` over that rung grows an arm in
lockstep (a missing arm is a compile error in the compiler — that is the safety net; follow it).

- **`Hir::Lambda { params: Vec<u32>, body: Box<Hir> }`** — a lambda. `params` are the fresh local ids
  resolve assigned its parameters (bound in the body's scope exactly like a function's params bind
  `Local(0..arity)`). Multi-arity to match the existing function/`Call` model (NOT curried in the IR — the
  spec's currying is a surface semantic realized by multi-arity application + compile-time **spine collapse**
  of a completed partial application into a direct call, NOT by declining; see §Subtleties + `fold.rs` #2).
- **`TypedNode::Lambda { params: Vec<u32>, body: Box<Typed> }`** — same shape; the node's `ty` is a
  `Ty::Fn(param_tys, ret)`.
- **`Mir::Lambda { params: Vec<u32>, body: Box<Mir> }`** — the transient value the fold reduces; a survivor
  declines in `select`. (Sibling to `Mir::FuncRef`/`Mir::Ctor`/`Mir::Intrinsic`.)
- **`Ty::Fn`** already exists (params → ret) with `core_valtype`/`comp_valtype` = `None`. **Increment A
  leaves this unchanged** — a `Ty::Fn` value is never runtime data. (Increment B is the only thing that
  ever makes it `I32`.)

No `Lir`/`serialize`/`Layout`/`heap` changes in Increment A — lambdas never reach the byte layer (they
reduce, or `select` declines before serialize).

---

## Pass-by-pass changes

### `resolve.rs` — `Ast → Hir`

Add one arm to `BodyResolver::form`, above the generic call/apply arms:

```
Some("fn") if items.len() == 3 => self.lambda(&items[1], &items[2], scope),
```

`lambda(param_form, body, scope)`:
- `param_form` is the parenthesized parameter list `(x)` / `(x y)` — collect names (a non-name → decline);
  allocate a fresh local id per param (`fresh_local`), chain a `Scope::Bind` frame per param (reuse
  `resolve_with_params`), resolve `body` under the extended scope, return `Hir::Lambda { params, body }`.
- The head-is-a-list Apply path (`resolve.rs:738`) already routes `((fn …) arg)` through `self.expr(head)`
  → this `fn` arm → so an *immediately-applied* lambda and a *bound* lambda both resolve with one arm.
- A local in head position `(g v)` already resolves to `Apply(Local(g), [v])` (`resolve.rs:897`) — the
  named-HOF path needs no resolve change.
- (Optional nicety: also accept a bare single param `(fn x body)`; the corpus only uses `(fn (p…) body)`.)

### `infer.rs` — `Hir → typed-Hir` (HM)

Add a `Hir::Lambda` arm to `Infer::expr`:
- fresh var per param; insert each into `self.locals`; infer the body under them; the lambda's type is
  `Ty::Fn(param_vars, body_ty)`; emit `TypedNode::Lambda`.
- This is textbook HM for `λ`; `Ty::Fn` + `unify`'s existing `Fn` arm (`ty.rs:477`) do the rest. The
  existing `Hir::Apply` arm (`infer.rs:262`, the general branch at 309–312) already unifies a `Fn`-typed
  callee against `Fn(argtys, freshret)` — **so applying a lambda already type-checks with no new code.**
- **⚠ Stop the eager under-arity decline — type a partial application as a `Ty::Fn` (REQUIRED for the
  curried-call case).** Today the `Hir::Call` arm declines when `args.len() < params.len()`
  (`infer.rs:187`, "partial application is a later phase"). That decline fires **before the fold runs**, so
  it kills `((add 3) 4)` (resolved as `Apply{ Call{add,[3]}, [4] }`) at the inner `Call{add,[3]}` and the
  fold never gets to collapse the spine. Change it: an under-arity `Call{f, args}` (or `Apply` of a
  `FuncRef`/`Lambda` to fewer args than its arity) **infers to a `Ty::Fn(remaining_param_tys, ret)`** — the
  type of the partial application — after unifying the supplied args against the leading params. It does NOT
  decline. This is standard curried typing: `(add 3) : Fn([Int], Int)`, then applying it to `4` unifies via
  the existing `Fn` arm and yields `Int`. The **fold** then decides emittability: a partial application
  completed to full arity at compile time collapses to a `Call` (see `fold.rs` #2); one that escapes
  survives as a `Ty::Fn` value and declines at `select` (Increment B). So the accept/decline line moves
  from **infer (eager, type-blind)** to **the fold (reduction-aware)** — exactly where the const-fold case
  the operator flagged is decided. (Over-application stays the CDZ0201 arity/type error it is today.)
- **Bidirectional seam (note, not required for A's corpus):** a bare un-applied lambda leaves its param
  vars unsolved; `ground`/`finalize` already default free vars *inside a `Fn`* to `Unit`
  (`infer.rs:802`), so an unreduced lambda grounds rather than false-declines — and then declines at
  `select` for the right reason. When check-mode against an expected `Fn` type lands (annotations /
  type-valued params), params take the expected types instead of fresh vars.
- Add a `Hir::Lambda` arm to `hir_uses_local` (`infer.rs:828`): a lambda uses a local iff its body does
  (its own params are locals it *binds*, but for the module-record pre-pass heuristic, treat "body uses a
  local" conservatively — a lambda body referencing a captured outer local counts).

### `lower.rs` — `typed-Hir → Mir`

Add `TypedNode::Lambda { params, body } => Mir::Lambda { params, body: lower(body) }`. Shape-preserving.
(The `TypedNode::Apply` arm is unchanged — an `Apply` whose callee lowers to a `Mir::Lambda` is handled by
the fold, exactly as `Apply(FuncRef)` is.)

### `fold.rs` — `eval` (the β-reduction — the heart of the change)

1. **`is_transient`** (`fold.rs:503`): add `Mir::Lambda { .. }`. So a `let`-bound lambda inlines into its
   body (`(let ((inc (fn …))) (inc 10))` → `Apply(Lambda,[10])`).
2. **`Mir::Apply` arm** (`fold.rs:307`) — two additions:
   - **`Mir::Lambda { params, body }` callee:** β-reduce — for each `(param, arg)` substitute `param` := arg
     into `body` (α-renamed; see infra below), then `self.fold(body)`. **Exact arity** β-reduces.
     **Over-application of a saturated lambda** (`((fn (x) …) a b)` where the body is not itself a function)
     is an apply-a-non-function type error (CDZ0201 upstream at infer).
   - **Spine collapse for CURRIED application (the partial-application-completed case):** an `Apply` whose
     `func` is *itself* an `Apply` of a `FuncRef`/`Lambda`/`Ctor` — i.e. `((f a) b)` lowered as
     `Apply(Apply(FuncRef(f),[a]),[b])` — must **gather the argument spine** and re-dispatch at the combined
     arity, NOT treat the inner `Apply` as an opaque value. Concretely: after folding `func`, if it folded
     to a residual `Apply(callee, inner_args)` (an under-applied partial application) AND `callee` is a
     `FuncRef`/`Lambda`, concatenate `inner_args ++ outer_args` and re-run the dispatch against `callee`:
       - `FuncRef(f)` at **full arity** (`arities[f]`) → `try_inline(f, all_args)` (const args fold to a
         value; runtime args stay a direct `Call{f, all_args}`) — **this is "curried → regular call"**;
       - `Lambda` at full arity → β-reduce as above;
       - still **under** full arity → leave the (now-flattened) residual `Apply(callee, gathered)` — a
         partial application whose completion is not visible here; the enclosing context may complete it, or
         it survives to select and declines (Increment B). Flattening the spine (vs nesting) is what lets an
         OUTER application complete it.
     The fold's `arities` table (already threaded via `Ctx`) supplies the target arity; no new state. A
     `FuncRef` applied to *fewer* args than its arity is thus NOT an eager decline — it is a partial
     application the fold keeps flattened, completed to a `Call` the moment the remaining args arrive at
     compile time. (⚠ this SUPERSEDES `infer.rs`'s current eager "partial application is a later phase"
     decline at `infer.rs:187` — see §"Accept / decline boundary" and the infer note below.)
3. **`try_inline`** (`fold.rs:371`): today it inlines only when *every arg `is_const`* and keeps the result
   only if `is_scalar_const || is_poison || is_module_record`. Extend both to make HOFs reduce:
   - **inline guard:** inline a non-recursive callee when every arg is a *compile-time value* — `is_const`
     **or `is_transient`** (a Lambda/FuncRef/Ctor/Intrinsic arg). This is what lets `(ap (fn …) 7)` inline
     `ap` and β-reduce `(g v)`. (A callee with a `Ty::Fn` parameter has no wasm param slot, so it **must**
     be inlined — see §Subtleties "function-typed parameters".)
   - **keep guard:** additionally keep the inlined result when it is a **transient function value**
     (a `Mir::Lambda`/`FuncRef`), so `((mk-adder 10) 5)` keeps the returned `Lambda` for the outer
     `Apply(Lambda,[5])` to β-reduce. (Do NOT keep a residual *runtime* value — that stays a `Call`, which
     then declines correctly if it carries a fn-typed value.)
4. **`substitute`** (`fold.rs:605`): add a `Mir::Lambda` arm (substitute into the body, respecting the
   lambda's own params as shadowing binders — like the `Let` shadow rule at `fold.rs:680`: do not
   substitute an id a lambda re-binds).
5. **`collect_calls`** (`fold.rs:722`) and **`collect_reached_poisons`** (`fold.rs:65`): add `Mir::Lambda`
   arms. For recursion detection, descend the lambda body (a lambda body can close a call cycle). For
   poisons, a lambda body is a **shielded** position (like an `if` branch / match arm) — do NOT descend it
   (an un-applied lambda body's trap is conditional on application), so a lambda body poison stays a
   shielded runtime trap, not an unconditional build failure.

**α-renaming infrastructure (the one non-trivial addition).** β-reduction and inlining splice a body
(a callee's, or a lambda's) into another scope. Local ids are assigned *per function* starting at 0
(`resolve` `next_local`), so a spliced body's ids can collide with the host scope's ids (e.g. inlining
`ap`'s `g=0`/`v=1` into `main`, whose lambda's `x` is also `0`). `select` keys `slot_of` by resolve-id
with no scope restore, so colliding ids **miscompile**. Fix: give the fold a **fresh-local supply** seeded
at `max_local_id_across_all_bodies + 1`, and **α-rename bound locals to fresh ids on every inline and
every β-reduction** before substituting. An `alpha_rename(mir, &mut remap, &mut supply)` helper rewrites
every *binding occurrence* — `Let.id`, `Lambda.params`, and `Match`-arm pattern `Local` binders — to a
fresh id, threading the remap into their bound uses. This is required for lambdas and **retroactively
hardens the existing const-inline path** (nested same-id `let`s after two inlines are a latent hazard
today). ⚠ Because it touches the proven const-inline path, validate α-renaming against the FULL corpus
(target: 0 regressions from 339) — see §Scope decisions for the always-vs-narrow choice.

### `select.rs` — `Mir → Lir`

- **Bare `Mir::Lambda`** (a survivor): add an arm returning
  `Err("a closure cannot yet cross to run time (runtime closures are a later increment)")` — the sibling of
  the existing `Mir::FuncRef`/`Mir::Ctor`/`Mir::Intrinsic` declines (`select.rs:176`). This is the
  Increment-A→B boundary made explicit.
- **`Mir::Apply { func: Mir::Lambda | Mir::FuncRef | Mir::Apply, .. }`** (a residual application the fold
  could not reduce — a partial application that never reached full arity at compile time, or a lambda
  selected by a runtime `if`): the `Mir::Apply` arm (`select.rs:199`) already declines any non-`Intrinsic`
  callee — extend its message to name the closure / incomplete-currying case. No new emission. (A curried
  application the fold DID complete never reaches here — it is a `Mir::Call`, emitted normally.)

---

## Accept / decline boundary (Increment A)

**Accept** (the fold reduces to emittable code):
- any `Apply(Lambda, exact-arity args)` where the reduction terminates in emittable Mir — including runtime
  captures reduced in place (`(+ 5 n)`), and lambdas passed to / returned from **non-recursive** helpers
  that the fold inlines;
- **a curried application completed to full arity at compile time** — `((add 3) 4)`, `(let ((a3 (add 3)))
  (a3 4))` — the fold collapses the spine to `Call{add,[3,4]}` (const args fold to a value; runtime args
  stay a direct call). This is the operator's "curried and const-folded into a regular function call" case;
- a lambda selected by a **const** `if`/`match` and then applied.

**Decline** (honest, never miscompile):
- a `Mir::Lambda` that **survives** to `select` — it escaped: stored in a **runtime** data structure,
  selected by a **runtime** `if`/`match` then applied, or applied through a **recursive** HOF the fold
  will not inline;
- a partial application that **never reaches full arity at compile time** — the completing argument is not
  compile-time-visible (it arrives inside a recursive HOF, from runtime data, or across a runtime branch),
  so the value stays an under-arity `Ty::Fn` and needs a runtime closure (Increment B). A partial
  application that IS completed at compile time does NOT decline — it is the accept case above.

**The tuple/list-storing corpus case** (`((tuple.0 (tuple (fn …) 9)) 5)`): today `Mir::Proj` folds only a
literal `Mir::Tuple` operand (`fold.rs:256`), and `is_module_record` (`fold.rs:536`) does not admit a
`Lambda` field, so the projection would not fold and the lambda would reach `select` and decline
(acceptable — the corpus permits declining). To make it **accept** in Increment A, generalize the
compile-time-projectable product: treat a `Mir::Tuple` **all of whose elements are compile-time values
(const or transient)** as projectable, so `Proj(slot)` yields the element (the `Lambda`), which the outer
`Apply` then β-reduces. Small, self-contained extension; do it if the case is in scope, else decline.

---

## The type-functions connection (task #150) — why build this now

The locked decision: **a type is a value; a generic is a type-valued parameter; monomorphization is
compile-time β-reduction = the same `eval` fold** ([[generics-are-type-valued-parameters]],
[[rcdzc-native-rewrite-phase0-landed]]; spec `type-system.md` §"Generics Are Type-Valued Parameters",
§"A Generic Definition Is Monomorphized … by compile-time reduction").

Therefore **a type function is a lambda whose parameters (and result) are types**:
- `(def (Pair A B) (Tuple A B))` is a 2-ary function returning a *type value*.
- `(Pair Int64 Bool)` is `Apply(<Pair>, [Int64, Bool])` — an **ordinary application** the fold β-reduces
  to the type value `(Tuple Int64 Bool)`. Same `Apply(Lambda,…)` β-reduction rule as a value closure.
- Because types are **compile-time-only and erased**, a type-lambda application **always** reduces at
  compile time — it is precisely the *pure Increment-A subset* and **never** hits Increment B. This is why
  first-class types (#150) can land on the same reduction tier without ever needing runtime closures.

**What this plan must guarantee for #150 to fall out cheaply:**
1. `Lambda`/`Apply`/β-reduction are **agnostic** to whether params/results are values or types — one
   reduction, no value/type fork.
2. The fold's *compile-time-value* predicate (the inline/keep guard) is phrased as "**is a compile-time
   value**" — const scalar, transient function value, **and (when #150 lands) a type value** — NOT the
   narrow "Int/Bool/Unit". Build the guard as `is_const || is_transient` now and widen it to admit type
   values in #150; do not hardcode scalar shapes.
3. The bidirectional seam (infer check-mode at type-valued-parameter positions) is where #150 plugs in;
   Increment A leaves the `Fn`-var-defaulting behavior (`infer.rs:802`) intact so an unapplied
   value-or-type lambda grounds rather than false-declines.

Net: **build the lambda + β-reduction tier for value closures (Increment A), and task #150 reuses it for
type functions with only the "compile-time value" predicate widened and the check-mode seam added.**

---

## Subtleties an implementer must get right

- **Inference runs BEFORE the fold.** So infer must type `Apply(Lambda, args)` correctly (unify lambda
  param vars with arg types) — it already does via the `Fn` unify arm. The fold only *reduces* an
  already-typed tree; it never re-derives a type (the discipline the old seed violated). Types are read
  off, not recomputed.
- **Function-typed parameters + DCE.** A helper like `ap : Fn([Fn(…), a], b)` has a `Ty::Fn` parameter,
  which has no `core_valtype` → `select_func` would error "parameter type unresolved" (`select.rs:106`).
  This is fine **only because** every call to `ap` passes a function value and is force-inlined, so `ap`
  becomes unreachable and `Layout` reachability drops it (`select_module` gives a dead func a placeholder,
  `select.rs:84`). Verify: (a) the inline guard force-inlines fn-typed-arg calls; (b) `Layout` reachability
  is computed over the **folded** module (it is — DCE landed with modules). A fn-typed-param helper that is
  *not* fully inlined (exported directly, or recursive) correctly declines at select — that is Increment B.
- **α-renaming is mandatory, not optional** (see infra above) — colliding resolve-ids across a splice
  miscompile at `select` (no `slot_of` scope restore).
- **Poison shielding:** a lambda body is a shielded position (do not collect its poisons unconditionally) —
  an un-applied lambda's trap is conditional on application, like an `if` branch.
- **Recursion detection must descend lambda bodies** (`collect_calls`) so a recursive closure is not
  wrongly inlined into non-termination.
- **Multi-arity vs currying:** realize `(fn (x y) body)` as a multi-arity `Lambda` (matches `Call`/
  `HirFunc`), apply with N args. The spec's curried desugaring (`(f a b)` ≡ `((f a) b)`) is a surface
  equivalence, not an IR requirement — do not build a curried IR (it would fork from the existing function
  model and complicate the type-function reuse). Instead, a curried/partial application is typed as a
  `Ty::Fn` at infer and **collapsed back to a direct multi-arg `Call` by the fold's spine-collapse** once
  it reaches full arity at compile time (the operator's flagged case). Under-application follows the new
  reduction-aware policy: keep a flattened residual, complete-to-`Call` when args arrive, decline only if
  it escapes (Increment B) — NOT the old eager decline. Over-application of a saturated value is CDZ0201.

---

## Scope decisions (recommendations for the implementer/operator)

1. **Increment split.** Recommend: implement **Increment A only** now (compile-time lambdas + β-reduction),
   which closes the entire core `09-functions.sexp` witness and lays the #150 substrate; architect but
   **defer Increment B** (runtime closures / `call_indirect` / heap env) — it is off the self-host critical
   path and a large lift. A declines B honestly.
2. **α-renaming blast radius.** Recommend: **always α-rename on inline and β-reduce** (correct and hardens
   the existing path), gated on a full-corpus green (0 regressions from 339). The narrower alternative
   (α-rename only on the lambda/β-reduce path, leave the proven const-inline path untouched) is lower-risk
   but leaves the latent nested-same-id hazard — acceptable only as a stopgap.
3. **Tuple/list-storing case.** Recommend: include the small compile-time-product projection generalization
   (§boundary) so the "function stored in a tuple element" corpus case *accepts* in A; if out of scope,
   decline it cleanly (the corpus permits it).

## Ladder placement

Increment A sits alongside task #150 (first-class types) as its shared reduction substrate — build the
β-reduction tier here, widen its compile-time-value predicate + add the check-mode seam in #150. Both
precede effects (#148). `Lir`/`serialize`/`Layout`/`heap` are untouched until Increment B.

Related: [[rcdzc-native-rewrite-phase0-landed]] (the transient-value + monomorphization=β-reduction
decisions), [[generics-are-type-valued-parameters]], the bidirectional-boundary learning (2026-07-04),
[[inference-plan-learn-from-seed-coarse-kind-mistakes]] (ask-75), `spec/semantics/09-functions.sexp` (the
witness this closes), `spec/capabilities/core-semantics.md` §Functions.
