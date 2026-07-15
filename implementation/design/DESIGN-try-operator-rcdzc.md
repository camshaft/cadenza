# Design — the `?` / `try` operator (fallible short-circuit) in `rcdzc`

**Author:** design agent (`design-try-operator`). **Audience:** the `vertical` agent that builds this,
plus `v-effects` / `v-inference` / `v-syntax`, who own seams it touches.
**Status:** **DESIGN ONLY — nothing landed.** Line numbers are landmarks at 2026-07-15, not promises.

> **Origin.** The operator asked for "a `?` operator like Rust — there are quite a few nested matches we
> could clean up" and, crucially, *"is that a monad generally? do we start going down that territory?"*
> This doc answers the crux first (§0) and then designs the narrow, high-value operator that answer
> permits.

## 0. The crux: `?` is NOT the thin end of a monad wedge — and Cadenza already decided that

The tempting reading is that `?` is Haskell's `do`-notation in disguise: `Result`/`Option`/`List` are all
monads, `?` is `>>=`, and adopting it commits Cadenza to a `Monad` abstraction (a higher-kinded constraint
variable, `bind`/`pure`, `do`-desugar). **We deliberately do not go there, and the decision was already
made before this operator came up:**

- [`spec/learnings/2026-07-04-traits-are-dictionaries-scoped-not-coherent.md`](../../spec/learnings/2026-07-04-traits-are-dictionaries-scoped-not-coherent.md)
  **defers higher-kinded constraint variables** ("abstracting over a constructor *in a constraint*
  (Functor/Monad) is where inference gets hard. Deferred deliberately") and **rejects global instance
  coherence**. A `Monad` class is exactly the HKT constraint that learning walks away from.
- Cadenza's chosen abstraction for **control** is **algebraic effects + handlers**, not monads
  ([DESIGN-effects-rcdzc.md](./DESIGN-effects-rcdzc.md)). Early-return-carrying-a-value is *already* a named
  thing there: the **abortive** arm class (§4.2 of that doc) — "bail and catch at the top." That machinery
  is landed and green (E0–E4 per the effects workstream).

So `?` is the **narrow ergonomic win** the operator wanted, built on machinery Cadenza already has, with
**zero new abstraction**: no `Monad`, no `do`-notation, no HKT, no new effect in the row/manifest. It kills
the nested-`match` boilerplate now and commits to nothing about the monad road.

**Before / after** (ML surface; application is `f(x)`, not juxtaposition):

```
-- today: nested match, one level of rightward drift per fallible step
def parse-pair(a: String, b: String) -> Result((Int64, Int64), Err) =
  match parse(a) with
  | Result.Err(e) => Result.Err(e)
  | Result.Ok(x)  =>
    match parse(b) with
    | Result.Err(e) => Result.Err(e)
    | Result.Ok(y)  => Result.Ok((x, y))

-- with `?`: flat, the happy path reads top-to-bottom
def parse-pair(a: String, b: String) -> Result((Int64, Int64), Err) =
  let x = parse(a)? in
  let y = parse(b)? in
  Result.Ok((x, y))
```

## 1. The one principle everything else follows from

**`?` is a compile-time desugar that borrows the effect system's *within-function abortive* discipline —
the short-circuit is discharged entirely at compile time, contributing nothing to the effect row.** There
is no user-visible `Try` effect, no `(effect …)` declaration, no handler the user writes. `e?` lowers to a
`match` on the operand whose **short-circuit arm yields the boundary's failure value** (§3.2); the boundary
is the enclosing function body (v1) or an explicit `try { }` region (v2). (An earlier draft framed the
short-circuit as a `break` to a synthesized `Mir::Block`; there is no such node in `Core` — see the
correction below.)

Three consequences fall out of this principle, and each one is load-bearing:

> **IMPLEMENTATION CORRECTION (2026-07-15, v-try-operator).** An earlier draft of this section named
> `Mir::Block` / `Mir::Break` as the lowering target, "landed for E4". That was **wrong**: rcdzc's `Core`
> IR has **no forward-break / labeled-block value node**, and the effects E4 work does **not** emit a
> runtime `block`/`br` — it discharges an abortive perform at **compile time** (`effects.rs`'s `abortive`
> set + `abort_value`). So `?` is **not** lowered to a block/break. The correct, equivalent realization
> (implemented) is a **compile-time match / short-circuit on the operand**: `e?` becomes a two-way choice
> on `e`'s discriminant — the success arm yields the payload, the failure arm yields the boundary's
> failure value. The text below has been updated to describe that; the *semantics* (§3.2) are unchanged.

1. **`?` always exits the *lexically* enclosing boundary** — the function it appears in, or the nearest
   enclosing `try`. That boundary is therefore **always in the same function body** at desugar time. So `?`
   is discharged entirely **within the function body**, exactly like the E4 within-function abortive
   handling ([DESIGN-effects-rcdzc.md §4.2](./DESIGN-effects-rcdzc.md), which folds the abort at compile
   time) — and **never** the cross-function non-local-exit calling convention. (A `?` in a helper exits the
   *helper*, exactly as Rust's does; it does not reach into the caller.)
2. **`?` contributes nothing to the effect row or the manifest.** Because the boundary is synthesized and
   discharged entirely at compile time (the fold selects the success/failure arm of a match on the
   operand), no effect label escapes. A program using `?` is *row-identical* to the desugared
   nested-`match` — it stays as pure as its body. `?` is therefore not the thin end of the *effect-row*
   wedge either.
3. **No new runtime, no new WIT, no backend change.** The lowering targets the **existing sum-match /
   `SumNew` Core** the compiler already emits for a hand-written `match` on `Result`/`Option` (a constant
   operand folds to the selected arm; a runtime operand emits a `Core::MatchSum`) — `select` already
   lowers both. There is **no** new `Core` node and **no** `select.rs` change. `?` is a front-half feature —
   surface + resolve + type + a Hir→Core desugar.

## 2. Surface (ML + s-expr + binary) — coordinate with `v-syntax`

Whatever the spelling, it must have a canonical s-expr and binary form and **round-trip on all three
surfaces** (`v-syntax` owns this; a garbage round-trip means it is not canonical — see the memory rule
*garbage-render-means-not-canonical*).

| Surface | Form | Notes |
|---|---|---|
| **ML** | `expr?` (postfix) | binds to the immediately preceding primary/postfix expression, highest precedence — `Map.lookup(env, k)?` binds `?` to the whole call, `x?.field` binds `?` before the field access. Rust-identical. |
| **s-expr (canonical)** | `(try expr)` | one operand; the machine form. |
| **binary** | a tagged `Try` node wrapping the operand | new node tag; `v-syntax` assigns it. |
| **ML `try` block** (v2) | `try { … }` → s-expr `(try-block …)` | an explicit boundary region (§4). |

The postfix `?` is the only new **lexer** token; `v-syntax` decides its exact precedence/associativity
(recommended: tighter than application-result, so it always postfixes the nearest complete expression).
`try` (v2) is a new **keyword** in the ML surface only; the s-expr uses the `(try-block …)` head (a reserved
word-string head, per the *compound-ctors-are-reserved-symbols* rule).

## 3. IR shapes and the desugar

### 3.1 A `Try` node, carried from Ast through resolve/infer, desugared at Hir→Core lowering

```rust
// A first-class resolved node (in rcdzc, `Resolved::Try`; sibling to the strict `Not`):
Try { operand: StructId }        // (try e)
// TryBlock { body: StructId }   // (try-block …)   -- v2, not yet added
```

`Try` survives resolve and infer as a first-class node (so type errors point at the `?`, not at desugared
guts), then is **desugared during the Hir→Core lowering** against the enclosing boundary.

### 3.2 The desugar (the whole semantics, in two rewrites)

Let `B` be the enclosing boundary and `T_B` its result type (the function's declared return type in v1, or
the `try` block's type in v2). `T_B` is either `Result(a, b)` or `Option(a)`. A `?` appears in a
CONTINUATION position — `e?` yields a value that the rest of the boundary body consumes; the canonical
shape is a let-initializer `(let ((x (try e))) <rest>)`, where `<rest>` is that continuation and produces
the boundary value. The rewrite makes the continuation the success arm and short-circuits on failure:

**A `Result`-typed operand** `e : Result(v, b)` — requires `T_B = Result(_, b)` (same error type `b`):

```
(let ((x (try e))) rest)   ==>   match e with
                                 | Result.Ok(x)  => rest         -- normal path: x = payload, run the continuation
                                 | Result.Err(r) => Result.Err(r) -- short-circuit: the boundary value IS the failure
```

**An `Option`-typed operand** `e : Option(v)` — requires `T_B = Option(_)`:

```
(let ((x (try e))) rest)   ==>   match e with
                                 | Option.Some(x) => rest
                                 | Option.None    => Option.None
```

The failure arm yields the operand's failure variant **unchanged** — and because `T_B` shares that failure
variant (the boundary is `Result(_, b)` / `Option(_)`, the same error type by the §5 check), the failure
value is just `e` itself re-emitted, needing no reconstruction. There is **no `break`/`Block`** — the
short-circuit is the failure ARM of an ordinary match, whose value flows out as the boundary's value
because the `?` sits in the boundary's tail continuation. The `Ok`/`Some` disc is read off the operand's
solved type (the built-in Option/Result variant discs, exactly as `List.at` / `Map.lookup` do today —
`option_discs`/`result_discs`, `lower.rs`).

### 3.3 How it lowers to Core

The rewrite targets the **existing sum-match Core** the compiler already emits for a hand-written `match`
on a `Result`/`Option`:

- A **constant** operand (`e` folds to a `Core::SumNew` — e.g. a checked-arith over constants → `Some v` /
  `None`) folds to the **selected arm** at compile time: success → the continuation with `x` bound to the
  payload; failure → the `SumNew` failure value. (This is the landed T1a path.)
- A **runtime** operand emits a `Core::MatchSum` on `e` (bound once): the success arm's continuation reads
  the payload via `Core::SumPayload`, the failure arm re-yields `e`. (T1b.)

Both are lowered by `select` already (a `match` on a sum is not new), so `?` adds **no new `Core` node and
no `select.rs` code**. The normal value is the continuation's value (`T_B`-typed — e.g. `Result.Ok((x,y))`)
and the short-circuit value is the operand's failure variant (also `T_B`), so the match is well-typed.

## 4. The boundary: function (v1) then `try { }` (v2)

The operator chose **both**, layered — the function boundary is the degenerate "the `try` wraps the whole
body" case.

**v1 — the enclosing function.** When a function body contains at least one `?` *not* inside a nested `try`
block, the boundary `T_B` is the function's **result type** (in Cadenza, the type of the function's body —
a return type is declared by ascribing the body `(: body T)`). No new surface — the annotation the user
already writes (`-> Result(a, b)` / `-> Option(a)`) is the boundary type. The `?` desugars into a match on
its operand whose failure arm's value flows out as the function's value (§3.2); there is no separate
"boundary block" node to synthesize — the enclosing function body *is* the boundary. A `?` in a function
whose result type is neither `Result` nor `Option` is a reject (§6, CDZ0230).

**v2 — an explicit `try { }` block.** `try { body }` is a boundary whose `T_B` is the block's own inferred /
checked `Result`/`Option` type, and whose *value* (when no `?` fires) is `body`. It lets a `?` be caught
*mid-function* and the result inspected without exiting the function:

```
def classify(a: String, b: String) -> String =
  let parsed = try {                 -- boundary: T_B = Result((Int64,Int64), Err)
    let x = parse(a)? in
    let y = parse(b)? in
    Result.Ok((x, y))
  } in
  match parsed with                  -- keep going in classify, whatever happened
  | Result.Ok(_)  => "both parsed"
  | Result.Err(_) => "at least one failed"
```

Nested `try` blocks nest boundaries; a `?` targets the **innermost** enclosing boundary (the same
dynamic-extent / nearest-enclosing rule handlers use — DESIGN-effects-rcdzc.md §3 "under-frame"). Because the
target is resolved lexically at desugar time, this is a plain innermost-block lookup, not a runtime search.

## 5. Typing — coordinate with `v-inference`

`?` is a **bidirectional check against a known boundary type**, which fits Cadenza's HM +
first-class-type-boundary shape ([`2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary.md`](../../spec/learnings/2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary.md)):

- Given boundary `T_B = Result(_, b)`: `e?` forces `e : Result(α, b)` (the **error type `b` unifies with the
  boundary's**) and yields `α`. All `?` sites under one boundary therefore unify their error types with each
  other and with `T_B` — HM does this for free.
- Given `T_B = Option(_)`: `e?` forces `e : Option(α)`, yields `α`.
- A `Result`-`?` under an `Option` boundary (or vice-versa) → type error. A `?` whose operand error type
  disagrees with `T_B`'s → type error. **No coercion** — see §5.1.
- v1's boundary type is the function's **declared** return type. An *unannotated* function that uses `?`
  still works if HM can infer a `Result`/`Option` return from the `?` sites + the body's tail, but the
  recommended (and doc'd) style is to annotate — the annotation is the boundary and makes the error type
  explicit at the seam.

### 5.1 No auto-conversion (against Rust's `?`-via-`From`)

Rust's `?` silently converts the error through the `From` trait. Cadenza **has no `From` and rejected
coherent trait resolution** (§0). So the error type must **match exactly**; a mismatch is a compile-time
reject, and the conversion is **explicit and visible** — the blessed idiom:

```
let y = Result.map-err(h(a), to-my-err)? in ...   -- convert, then ?
```

This is consistent with "an instance is a value you can see at the call" — no hidden dictionary, no hidden
conversion. **T3** ensures `Result.map-err` (and `Option.ok-or`, to lift an `Option` into a `Result` at a
boundary that wants one) exist in the prelude with docstrings, so the conversion path is a one-liner.

## 6. Diagnostics — register in `diag.rs::Code` (coordinate with `v-diagnostics`)

| Code | Meaning | Where |
|---|---|---|
| **CDZ0230** *(new)* | a `?` has **no fallible boundary that admits it** — the enclosing function/`try` result type is neither `Result` nor `Option`, or there is none. **Fix hint:** "annotate this function's return type as `Result(_, e)` / `Option(_)`, or wrap the expression in a `try { … }` block." | `infer::collect` (the boundary check ascends to the enclosing function body via `enclosing_boundary_ty`) |
| (CDZ0203) | the `?`'d type disagrees with the boundary — wrong error type, or `Result`-`?` under an `Option` boundary. The ordinary `TypeMismatch`; the fix hint names `Result.map-err` / `Option.ok-or` when a conversion would reconcile it. | infer |

CDZ0230 sits free in the CDZ02xx types-and-patterns band (0201/0202/0203, 0210–0214, 0220/0221 taken; 0230
open). It is produced in a checking pass over typed IR, **never** wedged into byte emission — the discipline
DESIGN-effects-rcdzc.md §7 insists on.

## 7. Increments (top-to-bottom, the way the vertical lands them)

Corpus lives in a **new** `spec/semantics/23-try-operator.sexp` (each stage names the cases it turns green;
promote passing breaker probes into it per the *breaker-promotes-passing-probes* rule).

- **T0 — surface + type + rejections (no lowering).** `v-syntax`: ML postfix `?` token, `(try e)` canonical
  s-expr, binary `Try` node, round-trip on all three. `rcdzc`: `Try` Hir leaf carried through resolve/infer;
  the bidirectional boundary-type check (§5); **CDZ0230** + the CDZ0203 mismatch. **Green:** the reject cases
  (`?` in a non-fallible function; `Result`-`?` under `Option`; mismatched error type) + the pure type-check
  cases. No value executes yet. **(Landed: T0a = the `Try` node through resolve/infer + operand-shape
  CDZ0203; T0b = the boundary check `enclosing_boundary_ty` + CDZ0230 + kind-mismatch CDZ0203.)**
- **T1 — function-boundary lowering (the core win).** Desugar `e?` per §3.2 for **both** `Result` and
  `Option` — a match on the operand whose success arm binds the payload and whose failure arm yields the
  operand's failure value (which flows out as the function's value; **no `Block`/`Break` node** — see the
  §1 correction). **Green:** the nested-`match`-collapse cases **executed through wasmtime** (a value comes
  out: happy path *and* short-circuit). **(Landed: T1a = the let-initializer `(let ((x (try e))) rest)`
  desugar for a CONSTANT-FOLDING operand — the two Option cases fold to their value. T1b = the RUNTIME
  operand via `Core::MatchSum` + a `(call …)` case that runs a value through wasmtime, and `?` in
  non-let-init positions — pending.)**
- **T2 — `try { }` block boundary (v2).** ML `try { … }` keyword + `(try-block …)` s-expr (round-trip);
  innermost-boundary resolution; the function boundary becomes the degenerate whole-body `try`. **Green:**
  mid-function catch cases (the `classify` shape); nested `try` blocks targeting the innermost.
- **T3 — the conversion idiom (prelude + docs).** Ensure `Result.map-err` and `Option.ok-or` exist in the
  prelude with docstrings; the CDZ0203 fix hint names them. **Green:** a case that converts an error type
  across a `?` boundary explicitly. (Small; may fold into T1/T2 if the prelude ops already exist.)

## 8. Seams / file anchors (landmarks at 2026-07-15)

- **Surface** — `implementation/seed/crates/cadenza-syntax/` (lexer token, ML↔s-expr↔binary, round-trip
  tests). **`v-syntax` territory.**
- **Ast node** — the `Resolved::Try { operand }` variant in `rcdzc/src/resolved.rs` (sibling to the strict
  `Not`); `resolve_try` in `resolve.rs` (mirrors `resolve_not`, arity-1). `try` is a control grammar head.
  (`TryBlock` for v2 is not yet added.)
- **Infer** — `rcdzc/src/infer.rs`: `fallible_shape` (classify `Option`/`Result` by variant names — no
  hard-coded key); `type_of(Try)` = the success payload; `enclosing_boundary_ty` (ascend to the enclosing
  function body — the boundary type is the body's type) + the `collect` boundary check (§5/§6). **`v-inference`
  reviews.**
- **Desugar → Core** — `rcdzc/src/lower.rs`: `try_let_desugar` (called from `lower_let`) rewrites `(let ((x
  (try e))) rest)` into a match on the operand — a constant `Core::SumNew` operand folds to the selected
  arm; a runtime operand emits `Core::MatchSum` (T1b). **No `Block`/`Break`.** The Option/Result variant
  discs are read off the solved type via `option_discs`/`result_discs` (`lower.rs`), exactly as `List.at` /
  `Map.lookup` do.
- **Select (unchanged)** — the sum-match / `SumNew` Core `?` targets is **already** lowered by `select`; `?`
  adds **no** `select.rs` code. If it seems to, the desugar targeted the wrong node.
- **Diagnostics** — `rcdzc/src/diag.rs::Code` (`TryNoBoundary` → CDZ0230 + its fix hint). **`v-diagnostics`
  reviews.**

## 9. The gate that protects it

Standard fleet gate (AGENTS-fleet.md §gate): `cargo test -p rcdzc --lib` (a desugar unit + a wasmtime run
where a `?` value executes, both happy and short-circuit path; a reject unit for CDZ0230), `cargo xtask
gate` (diff the FAIL SET — the new `23-try-operator.sexp` cases are additive; a `Todo→Fail` flip is a
miscompile), `cargo xtask check`. The **executing** wasmtime cases are the ones that matter — `?` is a
control construct, so a value must come out the far side, not merely type-check.

## 10. Decisions deferred (chosen defaults; don't pre-commit wording)

- **`?` on user sums shaped like `Result`/`Option`.** v1 is the **built-in** `Result`/`Option` only. A user
  `type MyResult = | Ok(a) | Err(b)` does not get `?` unless it *is* the built-in Result. Default: keep it
  built-in-only; revisit if a user asks — generalizing would want the "which variant is the short-circuit"
  to be declared, which edges toward a `Try`-like trait (an HKT-adjacent commitment we are avoiding).
- **`?` in tail position of the boundary.** `e?` as the *last* expression of a function returning `Result` —
  the `Ok(x)` arm yields `x`, but the boundary wants `Result`. Default: `e?` always *unwraps* (yields the
  payload), so a tail `e?` is a type error unless the payload type equals `T_B` — usually the user meant
  `e` (no `?`). The CDZ0203 hint should catch this ("drop the `?` to return the `Result` directly").
- **Precedence of postfix `?` vs field access / call chains.** `v-syntax`'s call; recommended Rust-identical
  (tightest postfix). Pinned by round-trip tests in T0.
- **A `try` *expression* without braces** (`try expr`, no block). Deferred — the block form (v2) subsumes it;
  add the braceless form only if the surface feels heavy in practice.
