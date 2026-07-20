# Design — the `?` and `try` operators (fallible short-circuit) in `rcdzc`

**Author:** design agent (`design-try-operator`). **Audience:** the `vertical` agent that builds this,
plus `v-effects` / `v-inference` / `v-syntax`, who own seams it touches.

> **🎨 RE-CHARTER (2026-07-17, operator directive).** `?` and `try` are **TWO DISTINCT operators**, not
> synonyms. The initial cut conflated them (`?` desugared to `(try e)`, both meaning the propagate); the
> operator has re-chartered them apart:
> - **`?` (a SUFFIX operator)** — the **propagate / short-circuit**: `expr?` unwraps a success payload, or
>   ABORTS (bubbles the failure) to the nearest enclosing boundary. This is what the *old* `try` did, and
>   matches Rust's `?`.
> - **`try { … }` (a CATCHER scope)** — a **delimited boundary** that BOUNDS where an inner `?` aborts to,
>   and **returns that block's `Result`/`Option`** (it catches the `?` before it bubbles to the function
>   top). `try` is the boundary a `?` unwinds to, NOT a synonym for `?`.
>
> This is the v1/v2 model §4 already describes, with the names split: v1 = the enclosing-FUNCTION boundary a
> bare `?` targets; v2 = an explicit `try { }` boundary. It is a **nested-`Core::Block` + nearest-enclosing
> target** change, **not new IR** — `Core::Block`/`Break` already model a boundary + non-local break to the
> nearest enclosing block. Pending operator confirmation of 4 forks (recorded §4.1): (A) `try` is an
> **expression** returning the block's Result [rec]; (B) **no auto-wrap** — the `try` body must be
> `Result`/`Option`-typed [rec, per §5.1]; (C) the propagate op needs its own **s-expr head** now that
> `(try e)` = the catcher — lean **`(? e)`**; (D) **two independent** error-unification scopes (inner `try`
> vs the function). Sections below are being revised to this model; where they still say "`?`/`try`
> synonyms" read `?` = propagate, `try` = catcher.

**Status (2026-07-20):** **LANDED (as the propagate op, under the old `try` spelling); UNDER REDESIGN (name
split).** On trunk today: T0a (the `Try` node through resolve/infer + operand-shape CDZ0203), T0b (boundary
check `enclosing_boundary_ty` + CDZ0230 + kind-mismatch CDZ0203, incl. the error-type-agreement CDZ0203),
BRICK 1 (`Core::Block`/`Break` nodes), BRICK 2a (constant-success fold), BRICK 3a (constant-failure
short-circuit + §283 elide). These implement the **propagate** semantics (the future `?`) at the function
boundary. Comprehensively gated (`spec/semantics/23-try-operator.sexp` + the rcdzc `try` unit tests): the
constant-fold executing paths (success/failure, inline-no-let, let-bound-var operand, compound payload,
if/match position, anonymous-lambda boundary), the reject family (CDZ0203 operand-shape / kind-mismatch /
error-type-disagree, CDZ0230 no-boundary), the strict-spine effect ordering, and two rcdzc-lib wasmtime RUN
tests (a value executes, both paths). The lambda boundary is ruled (§6 v1): a lambda IS a function boundary,
no auto-wrap. A diagnostic-dedup fix drops the misleading "lowers only a constant operand yet" decline on an
ill-typed (CDZ0203) operand. REMAINING: the name split (`?` suffix vs `try` catcher, §4.1 forks); the RUNTIME
`?` (non-constant operand → `Core::MatchSum`/block-br emit, BRICK 3b, operator-gated); the ML postfix `?`
surface + the `(? e)` s-expr head (v-syntax); the explicit `try { }` catcher boundary (v2 / §4); and the T3
conversion-idiom prelude ops (`Result.map-err`/`Option.ok-or`). Line numbers are landmarks, not promises.

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
the short-circuit is discharged entirely within the enclosing function, contributing nothing to the effect
row.** There is no user-visible `Try` effect, no `(effect …)` declaration, no handler the user writes. The
boundary is the enclosing function body (v1) or an explicit `try { }` region (v2); `e?` unwraps the success
payload and, on the failure variant, **breaks to the enclosing boundary** carrying the failure value (§3.2).

Three consequences fall out of this principle, and each one is load-bearing:

> **IMPLEMENTATION NOTE (2026-07-15, v-try-operator).** History for readers: this doc originally named
> `Mir::Block` / `Mir::Break` as the lowering target "landed for E4". At the time that was inaccurate —
> rcdzc's `Core` IR had **no** forward-break/labeled-block node, and E4 discharges its abortive performs at
> **compile time** (`effects.rs`'s `abortive` set + `abort_value`), not via a runtime `block`/`br`. The
> project's resolution was to **ADD the nodes**: `Core::Block { result_ty, body }` and
> `Core::Break { value }` are now in `core.rs` (landed as **BRICK 1**), lowered in bricks — **BRICK 2** the
> Hir→Core desugar (`e?` → success-payload / `Core::Break`, boundary body wrapped in `Core::Block`) and
> **BRICK 3** the wasm `block`/`br` emit. A separate **BRICK 2a** fast path folds a *constant-success*
> `(try (Some x))` straight to its payload at the `Try` node (no boundary break needed on the happy path).
> So the design below (a boundary `Core::Block` + a `Core::Break` short-circuit) is EXACTLY what is being
> built; `Mir::*` is just the old spelling of `Core::Block`/`Core::Break`.

1. **`?` always exits the *lexically* enclosing boundary** — the function it appears in, or the nearest
   enclosing `try`. That boundary is therefore **always in the same function body** at desugar time. So `?`
   is discharged entirely **within the function body**, exactly like the E4 within-function abortive
   handling ([DESIGN-effects-rcdzc.md §4.2](./DESIGN-effects-rcdzc.md), which folds the abort at compile
   time) — and **never** the cross-function non-local-exit calling convention. (A `?` in a helper exits the
   *helper*, exactly as Rust's does; it does not reach into the caller.)
2. **`?` contributes nothing to the effect row or the manifest.** Because the boundary block is synthesized
   and discharged entirely within the function (a `Core::Block`/`Break` pair the backend lowers to a
   `block`/`br`), no effect label escapes. A program using `?` is *row-identical* to the desugared
   nested-`match` — it stays as pure as its body. `?` is therefore not the thin end of the *effect-row*
   wedge either.
3. **No new runtime, no new WIT.** The `Core::Block`/`Break` nodes lower to a plain wasm `block`/`br` — no
   runtime support, no WIT change; the backend arm (BRICK 3) is the only `select.rs` addition, and it is
   the same `block`/`br` machinery structured control already implies. `?` is a front-half feature —
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
the `try` block's type in v2). `T_B` is either `Result(a, b)` or `Option(a)`.

**A `Result`-typed operand** `e : Result(v, b)` — requires `T_B = Result(_, b)` (same error type `b`):

```
e?   ==>   match e with
           | Result.Ok(x)  => x                        -- normal path: yields the payload
           | Result.Err(r) => break B (Result.Err(r))  -- abortive: the boundary's value becomes Err(r)
```

**An `Option`-typed operand** `e : Option(v)` — requires `T_B = Option(_)`:

```
e?   ==>   match e with
           | Option.Some(x) => x
           | Option.None(_) => break B (Option.None)
```

`break B w` is `Core::Break { value: w }` targeting the `Core::Block` synthesized for boundary `B` (§4) —
both nodes landed as **BRICK 1** (`core.rs`). The `Ok`/`Some` disc is read off the operand's solved type
(the built-in Option/Result variant discs, exactly as `List.at` / `Map.lookup` do today via
`option_discs`/`result_discs`, `lower.rs`).

**Constant-success fast path (BRICK 2a, landed).** When `e` folds to a compile-time-CONSTANT success
variant (`Some x` / `Ok x`), `?` unwraps straight to the payload at the `Try` node — no boundary break
fires on the happy path — so a constant-`Some`/`Ok` `?` needs no `Core::Block`/`Break` at all (it folds
like a constant `List.at`). The `Block`/`Break` path (BRICK 2/3) is for the FAILURE arm and the RUNTIME
operand.

**Constant-failure fast path (BRICK 3a, landed) — and the §283 elide ruling.** When `e` folds to a
compile-time-CONSTANT failure variant (`None` / `Err r`), the `?` short-circuits the WHOLE continuation:
`lower_let` recognizes a let-init that is a `Try` lowering to `Core::Break { value }` and folds the enclosing
let straight to `core_of(value)` — the boundary result is that failure value, and every later binding (and
the body) is dropped, since the break makes them unreachable. **This is where the §283 ruling bites:** an
earlier let-init that *would* trap but whose value is never observed on the surviving path is NOT forced to
trap — the trap is ELIDED (the binding compiles to the failure value, e.g. `(None unit)`, with a CDZ0305
"detected-unreachable-trap" WARNING). Operator ruling (verbatim intent): *"we don't emit the trap unless it's
reachable; but if we detect an unreachable trap it should be a warning."* This makes the same-let, nested-let,
and `if false` shapes all consistently elide, aligning with the landed §283 dead-binding DCE. The ONE guard
that survives on this fast path is **host-call-freedom** (`subtree_reaches_host_call`): a host call / `perform`
IS an observable effect §283 lists, so a discarded init that reaches one bails the fold (no elide) rather
than dropping an observable effect. There is NO `is_trap_free` guard here — a pure trap on an unobserved value
elides. Corpus pins: `23-try-operator.sexp` same-let + nested-let discard cases (→ `(None unit)` + CDZ0305),
and the RUNTIME-effect observability cases (effectful-init-before-failing-`?` performs exactly once).

### 3.3 The boundary block

A boundary `B` with body `body` and result type `T_B` lowers to:

```
Core::Block { result_ty: T_B, body: <body, with each contained e? desugared as in §3.2> }
```

The block's **normal fallthrough value** is `body`'s value (already `T_B`-typed — e.g. `Result.Ok((x,y))`);
its **break value** is whatever a `?` inside supplied (`Result.Err(r)` / `Option.None`). Both are `T_B`, so
the block is well-typed. Unit-free, no state, no continuation object — the E4 abortive shape with `init`
unused, a `block`/`br` pair the backend emits (**BRICK 3**; until it lands, `select` declines a
`Core::Block`/`Break`, so a failure/runtime `?` is a clean Todo, never wrong code).

## 4. The boundary: function (v1) then `try { }` (v2)

The operator chose **both**, layered — the function boundary is the degenerate "the `try` wraps the whole
body" case.

**v1 — the enclosing function.** When a function body contains at least one `?` *not* inside a nested `try`
block, the lowering wraps the whole body in a `Core::Block` whose `T_B` is the function's **result type**
(in Cadenza, the type of the function's body — a return type is declared by ascribing the body
`(: body T)`, so the body's type IS the result type). No new surface — the annotation the user already
writes (`-> Result(a, b)` / `-> Option(a)`) is the boundary type. Each `?` inside becomes a `Core::Break`
to that block on the failure arm (§3.2). A `?` in a function whose result type is neither `Result` nor
`Option` is a reject (§6, CDZ0230).

*"Function" here means any function body, named OR anonymous.* An immediately-applied lambda
(`((fn () (let ((x (try (Some 7)))) (Some (+ x 1)))))`) is a boundary exactly like a named `def`:
`enclosing_boundary_ty` walks to the nearest enclosing `(fn params body)` body, not only a `def` body. There
is **no auto-wrap** (§5.1 fork B, decided): a lambda's result type is NOT promoted to `Option`/`Result` just
because its body has a top-level `?` — the boundary is the lambda's *actual* result type. So a `?` under a
FALLIBLE-typed lambda works (success unwraps, failure short-circuits the lambda), and a `?` under a
NON-fallible lambda result is CDZ0230, the *same* rule as a def body — one rule for every function body, no
def-vs-lambda divergence. (Ruled by v-try-operator 2026-07-20; the fallible-lambda executing behavior is
corpus-pinned. The non-fallible-lambda CDZ0230 was initially missed for an *applied* lambda — the `?` was
checked only on the β-reduced inlined copy, whose parentless boundary-walk hit the inlined-helper
inconclusive tolerance — and is enforced by descending the original parented applied-lambda body; see the
`try-op-in-applied-anon-lambda` fix.)

*Accepted limitation — a `?`-bodied lambda passed as a HIGHER-ORDER ARGUMENT.* The lambda-result-typing rule
above is bottom-up and identical to a `def`: a bare `(fn (x) (try (Some x)))` types its result as the
UNWRAPPED payload (`(-> Int64 Int64)`), NOT the fallible type — because auto-wrapping the result to `Option`
would be exactly the §5.1 fork-B magic this design rejects, and would diverge a lambda from its `def` twin
(whose bare-`?` body is `CDZ0230`/`CDZ0203`, not wrapped). A useful consequence: passed to a HOF expecting a
FALLIBLE result (`(-> Int64 (Option Int64))`), such a bare-`?` lambda is a clear **CDZ0203** arrow-result
mismatch, and the blessed idiom — RE-WRAP the tail (`(fn (x) (let ((y (try (Some x)))) (Some y)))`) — types
`(-> Int64 (Option Int64))` and works. The one un-caught corner: a bare-`?` lambda passed to a HOF expecting
a NON-fallible result (`(-> Int64 Int64)`) *type-matches* that slot and compiles silently (its `?` unwraps),
where the named-helper twin would be `CDZ0230`. This is a knowingly-**accepted low-severity limitation** (a
missing *rejection* on an uncommon shape, never a wrong-value miscompile): forcing `CDZ0230` there needs a
use-site-arrow boundary read whose false-positive hazard on generic/uninstantiated HOF-arg lambdas outweighs
rejecting a shape the author gets a correct `CDZ0203` for the moment the slot is given a fallible type (the
useful case). Ruled by v-try-operator 2026-07-20; the immediately-applied fix (`db0e6a723`) is the right
scope, deliberately not widened to HOF-arg.

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

A **bare `?` outside any `try`** targets the function boundary (v1) — unchanged from today. So splitting the
names is **backward-preserving** for existing `?`-in-function programs: they keep aborting to the function.
`try { }` only adds the ability to catch a `?` at an inner boundary.

### 4.1 Open design forks (routed to the operator 2026-07-17; recommendations are the proposed defaults)

The re-charter raises four semantics questions. Each is written with the vertical's recommendation; the
operator confirms or tweaks before code lands.

- **(A) `try` as expression vs statement.** *Recommend EXPRESSION* — `try { … }` evaluates to the block's
  `Result`/`Option`, so it composes: `let parsed = try { … } in match parsed …` (the §4 example). Matches
  the operator's "returns the result from that closure."
- **(B) What `try` returns on the no-`?`-fires path.** *Recommend NO AUTO-WRAP* — the `try` body's tail
  must already be `Result`/`Option`-typed (e.g. `Result.Ok((x,y))`); `try` does not silently wrap a bare
  success into `Ok`. Consistent with the no-coercion stance (§5.1): the wrap is explicit and visible.
- **(C) The propagate op's s-expr head.** `?` is postfix in ML, but the canonical s-expr surface needs a
  head, and `(try e)` now denotes the **catcher**, not the propagate. *Recommend `(? e)`* as the propagate
  head (mirrors the ML suffix); `(try BODY…)` becomes the catcher block. (Alternative floated: `(propagate
  e)`.) v-syntax owns the postfix `?` token + this head — coordinate at T-R1.
- **(D) Error-type unification scope.** *Recommend TWO INDEPENDENT scopes* — within a `try`, all `?` error
  types unify with the `try`-block's `Result` error (§5); a bare `?` in the same function still unifies with
  the **function** result error. Two lexical boundaries ⇒ two unification scopes, the natural reading. HM
  does each scope's unification for free (§5).

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
- **T1 — function-boundary lowering (the core win), built in bricks:**
  - **BRICK 1 (landed):** the `Core::Block { result_ty, body }` + `Core::Break { value }` nodes + their
    non-emit pass-through arms; the backend declines the emit for now.
  - **BRICK 2a (landed):** the constant-success fast path — `(try (Some x))` / `(try (Ok x))` folds to its
    payload at the `Try` node (the "success unwraps the payload" corpus case → pass).
  - **BRICK 2 (this vertical):** the Hir→Core desugar — wrap a fallible fn body containing a `?` in a
    `Core::Block { result_ty: T_B }`, rewrite each `e?` to payload-on-success / `Core::Break` on the
    failure arm (§3.2). Boundary discovery = `enclosing_boundary_ty` (from T0b).
  - **BRICK 3:** the `select.rs` `block`/`br` emit. **Green (after 3):** the failure-short-circuit +
    runtime `?` cases **executed through wasmtime** (a value comes out on both the happy and short-circuit
    path).
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
- **Core nodes** — `Core::Block { result_ty, body }` + `Core::Break { value }` in `rcdzc/src/core.rs`
  (landed as **BRICK 1**, with the pass-through arms in `select.rs`). `success_disc_of` (`lower.rs`) reads
  the success disc off the operand's solved Option/Result type.
- **Desugar → Core (BRICK 2)** — `rcdzc/src/lower.rs`: wrap a fallible function body containing a `?` in a
  `Core::Block { result_ty: T_B }`; rewrite each `e?` to the payload on success and a `Core::Break` on the
  failure arm (§3.2). A **constant-success** `(try (Some x))` short-cuts to its payload at the `Try` node
  (BRICK 2a, landed) — no block needed. Boundary discovery: `enclosing_boundary_ty` (from T0b).
- **Select (BRICK 3)** — `Core::Block`/`Break` → wasm `block`/`br` in `select.rs` (until then a
  `Core::Block`/`Break` cleanly DECLINES — the stub at the `Core::Block { .. } | Core::Break { .. }` arm).
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
