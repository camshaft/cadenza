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

**`?` is a compile-time desugar to the effect system's *within-function abortive lowering* — it borrows the
lowering substrate, not the surface.** There is no user-visible `Try` effect, no `(effect …)` declaration,
no handler the user writes. `e?` lowers to a `match` whose short-circuit arm is an abortive **break** to a
compiler-synthesized **boundary block**; the boundary is the enclosing function body (v1) or an explicit
`try { }` region (v2).

Three consequences fall out of this principle, and each one is load-bearing:

1. **`?` always exits the *lexically* enclosing boundary** — the function it appears in, or the nearest
   enclosing `try`. That boundary is therefore **always in the same function body** at desugar time. So `?`
   only ever needs the **cheapest** abortive rung — the within-function `block`/`br` of
   [DESIGN-effects-rcdzc.md §4.2](./DESIGN-effects-rcdzc.md) (`Mir::Block` / `Mir::Break`) — and **never**
   the cross-function non-local-exit calling convention. (A `?` in a helper exits the *helper*, exactly as
   Rust's does; it does not reach into the caller.)
2. **`?` contributes nothing to the effect row or the manifest.** Because the boundary is synthesized and
   discharged entirely at compile time (a `block`/`br` pair the fold produces), no effect label escapes. A
   program using `?` is *row-identical* to the desugared nested-`match` — it stays as pure as its body. `?`
   is therefore not the thin end of the *effect-row* wedge either.
3. **No new runtime, no new WIT, no backend change.** The lowering targets `Mir::Block` / `Mir::Break`,
   which `select` already maps to wasm `block` / `br` (landed for E4). `?` is a front-half feature —
   surface + resolve + type + a Hir→Mir desugar.

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

### 3.1 A `Try` node, carried from Ast to Hir, desugared at Hir→Mir

```rust
// Ast / Hir gain a leaf (sibling to the other control forms):
Try { operand: Box<_> }          // (try e)
TryBlock { body: Box<_> }        // (try-block …)   -- v2
```

`Try` survives resolve and infer as a first-class node (so type errors point at the `?`, not at desugared
guts), then is **desugared during the Hir→Mir lowering** against the enclosing boundary.

### 3.2 The desugar (the whole semantics, in two rewrites)

Let `B` be the enclosing boundary and `T_B` its result type (the function's declared return type in v1, or
the `try` block's type in v2). `T_B` is either `Result(a, b)` or `Option(a)`.

**A `Result`-typed operand** `e : Result(v, b)` — requires `T_B = Result(_, b)` (same error type `b`):

```
e?   ==>   match e with
           | Result.Ok(x)  => x                       -- normal path: yields the payload
           | Result.Err(r) => break B (Result.Err(r)) -- abortive: the boundary's value becomes Err(r)
```

**An `Option`-typed operand** `e : Option(v)` — requires `T_B = Option(_)`:

```
e?   ==>   match e with
           | Option.Some(x) => x
           | Option.None(_) => break B (Option.None)
```

`break B w` is `Mir::Break{ value: w }` targeting the `Mir::Block` synthesized for boundary `B` (§4). The
`Ok`/`Some` disc is read off the operand's solved type (the built-in Option/Result variant discs, exactly as
`List.at` / `Map.lookup` do today — `lower.rs:1552`, `lower.rs:1808`).

### 3.3 The boundary block

A boundary `B` with body `body` and result type `T_B` lowers to:

```
Mir::Block { result_ty: T_B, body: <body, with each contained e? desugared as in §3.2> }
```

The block's **normal fallthrough value** is `body`'s value (already `T_B`-typed — e.g. `Result.Ok((x,y))`);
its **break value** is whatever a `?` inside supplied (`Result.Err(r)` / `Option.None`). Both are `T_B`, so
the block is well-typed. Unit-free, no state, no continuation object — this is the E4 abortive shape with
`init` unused.

## 4. The boundary: function (v1) then `try { }` (v2)

The operator chose **both**, layered — the function boundary is the degenerate "the `try` wraps the whole
body" case.

**v1 — the enclosing function.** When a function body contains at least one `?` *not* inside a nested `try`
block, the lowering wraps the whole body in a boundary block whose `T_B` is the function's **result type**.
No new surface — the annotation the user already writes (`-> Result(a, b)` / `-> Option(a)`) is the boundary
type. A `?` in a function whose result type is neither `Result` nor `Option` is a reject (§6).

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
| **CDZ0230** *(new)* | a `?` has **no fallible boundary that admits it** — the enclosing function/`try` result type is neither `Result` nor `Option`, or there is none. **Fix hint:** "annotate this function's return type as `Result(_, e)` / `Option(_)`, or wrap the expression in a `try { … }` block." | Hir→Mir desugar / a resolve-time boundary check |
| (CDZ0203) | the `?`'d type disagrees with the boundary — wrong error type, or `Result`-`?` under an `Option` boundary. The ordinary `TypeMismatch`; the fix hint names `Result.map-err` / `Option.ok-or` when a conversion would reconcile it. | infer |

CDZ0230 sits free in the CDZ02xx types-and-patterns band (0201/0202/0203, 0210–0214, 0220/0221 taken; 0230
open). It is produced in a checking pass over typed IR, **never** wedged into byte emission — the discipline
DESIGN-effects-rcdzc.md §7 insists on.

## 7. Increments (top-to-bottom, the way the vertical lands them)

Corpus lives in a **new** `spec/semantics/22-try-operator.sexp` (each stage names the cases it turns green;
promote passing breaker probes into it per the *breaker-promotes-passing-probes* rule).

- **T0 — surface + type + rejections (no lowering).** `v-syntax`: ML postfix `?` token, `(try e)` canonical
  s-expr, binary `Try` node, round-trip on all three. `rcdzc`: `Try` Hir leaf carried through resolve/infer;
  the bidirectional boundary-type check (§5); **CDZ0230** + the CDZ0203 mismatch. **Green:** the reject cases
  (`?` in a non-fallible function; `Result`-`?` under `Option`; mismatched error type) + the pure type-check
  cases. No value executes yet.
- **T1 — function-boundary lowering (the core win).** Synthesize the boundary block around a function body
  containing `?`; desugar `e?` per §3.2 for **both** `Result` and `Option`; lower via `Mir::Block` /
  `Mir::Break` (E4 substrate, already in `select`). **Green:** the nested-`match`-collapse cases — the
  `parse-pair` shape (Result) and a `head`/`lookup` chain (Option) — **executed through wasmtime** (a value
  comes out: the happy path *and* a short-circuit path). This is the increment that delivers the operator's
  ask.
- **T2 — `try { }` block boundary (v2).** ML `try { … }` keyword + `(try-block …)` s-expr (round-trip);
  innermost-boundary resolution; the function boundary becomes the degenerate whole-body `try`. **Green:**
  mid-function catch cases (the `classify` shape); nested `try` blocks targeting the innermost.
- **T3 — the conversion idiom (prelude + docs).** Ensure `Result.map-err` and `Option.ok-or` exist in the
  prelude with docstrings; the CDZ0203 fix hint names them. **Green:** a case that converts an error type
  across a `?` boundary explicitly. (Small; may fold into T1/T2 if the prelude ops already exist.)

## 8. Seams / file anchors (landmarks at 2026-07-15)

- **Surface** — `implementation/seed/crates/cadenza-syntax/` (lexer token, ML↔s-expr↔binary, round-trip
  tests). **`v-syntax` territory.**
- **Ast/Hir node** — the `Try` / `TryBlock` leaves in `rcdzc/src/core.rs` (sibling to the other control
  nodes).
- **Resolve / boundary tracking** — `rcdzc/src/resolve.rs`: track the nearest enclosing fallible boundary
  (function result type or `try` block) so a stray `?` is caught as CDZ0230.
- **Infer** — `rcdzc/src/infer.rs`: the bidirectional check of §5. **`v-inference` reviews.**
- **Desugar → Mir** — `rcdzc/src/lower.rs`: emit the `Mir::Block` boundary + rewrite each `?` to
  `match … | short => Mir::Break`. The Option/Result variant discs are read off the solved type exactly as
  `List.at` (`lower.rs:1552`) / `Map.lookup` (`lower.rs:1808`) do.
- **Select (unchanged)** — `Mir::Block`/`Mir::Break` → `block`/`br` already exists from E4; `?` should add
  **no** `select.rs` code. If it seems to, the desugar targeted the wrong node.
- **Diagnostics** — `rcdzc/src/diag.rs::Code` (CDZ0230 + its fix hint). **`v-diagnostics` reviews.**

## 9. The gate that protects it

Standard fleet gate (AGENTS-fleet.md §gate): `cargo test -p rcdzc --lib` (a desugar unit + a wasmtime run
where a `?` value executes, both happy and short-circuit path; a reject unit for CDZ0230), `cargo xtask
gate` (diff the FAIL SET — the new `22-try-operator.sexp` cases are additive; a `Todo→Fail` flip is a
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
