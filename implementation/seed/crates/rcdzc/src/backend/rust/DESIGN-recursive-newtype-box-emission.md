# DESIGN: recursive-newtype un-erasure (Box-indirected nominal emission) on the Rust backend

Status: **PROPOSED** (design pass; no code lands with this doc). Owner: `v-rust-backend`.

## The gap

Two corpus cases (`spec/semantics/05-compound-types.sexp`) run on wasm but DECLINE on the Rust
backend:

- `a recursive NEWTYPE-wrapped linked list folds to a scalar`
- `a recursive NEWTYPE traversal recurses on its projected recursive field`

Both use the single-variant recursive newtype

```
(type Lst (Mk (Option (Tuple Int64 Lst))))
(def (sm (: l Lst)) (match l ((Mk o) (match o ((Some p) (+ (. p 0) (sm (. p 1)))) ((None) 0)))))
```

After the parity audit (`comm` of rust-`todo` vs wasm-`pass` on `.gate-baseline`, minus the
peer-lane categories — effects/host/@param/Ast/rope/slice/closure), the recursive newtype is the
**only** remaining pure-value rust-only gap. (The one other survivor, "a mutually-recursive decoder
returns a heap value and cursor", declines on `(List Any) has no native Rust representation` — an
unsolved element-type, an inference/`ground_open_vars` concern, NOT a rust-emit unit.)

## Why it declines today (empirically confirmed)

A recursive newtype **erases at the type level** — `infer::newtype_underlying` (infer.rs, "Phase 2")
returns an `inner` even when the payload mentions the decl's own name, and `Db` construction keys it
in `db.newtype_inner` (db.rs). Its `inner` is finite: the self-reference decodes to a bare
`Ty::Sum { decl }` μ-leaf (`Option (Int64, Ty::Sum{Lst})`), not an infinite unfolding.

On the Rust backend that erasure has nowhere to land:

1. `types::rust_type(Ty::Nominal { inner, .. })` (types.rs:112) recurses into `inner` →
   `rust_type` reaches the bare `Ty::Sum { name: "Lst" }` leaf → renders the type name `Lst`.
2. But `enums::emit_one_enum` **declines every newtype unconditionally** — `if
   db.newtype_inner.contains_key(&decl.occ) { return Err("an erased newtype has no boxed enum") }`
   (enums.rs) — so no `enum Lst`/`struct Lst` is ever emitted. Naming `Lst` references an undefined
   type ⇒ an uncompilable crate.
3. The backend therefore declines at the construct/match site: `sum_variant_path_of_ty`
   (expr.rs) gates on `sum_representable` and returns a decline. Since 2026-07-29 that decline names
   the precise reason via `enums::unrepresentable_reason` → **"a recursive newtype with no
   Box-indirected Rust representation"** (landed `fc4eb13a1`). That message is the breadcrumb this
   doc picks up.

wasm has no analogous problem: a nominal erases to a heap handle, no named type is emitted, and the
μ back-edge is just another heap pointer.

## The working analogue: recursive SUMS

A recursive multi-variant **sum** already emits and runs on Rust (`.gate-baseline-rust` shows
`a general recursive-sum recursion (Nil|Cons count)`, `recursive-sum equality …`, etc. all `pass`).
`emit_one_enum` handles it: a variant whose payload `variant_payloads_mention`s the decl BOXES the
whole payload field with a fully-qualified `::std::boxed::Box<…>` (enums.rs) — one box per recursive
variant, so a Rust enum containing itself stays finite-sized (avoids E0072). The construct site
emits `Box::new(payload)` and the match site derefs `*__pay`; both agree on the one-box scheme.
`derive(PartialEq, Eq, PartialOrd, Ord)` composes over `Box<T: Eq>` fields, so `=`/compare still
work.

A recursive newtype is the **single-variant** instance of exactly this shape. The machinery exists;
it is simply gated off for anything in `newtype_inner`.

## The hard constraint: `newtype_inner` is a cross-backend invariant

`db.newtype_inner` is not a rust concern — it is the shared contract that **every reader agrees on
the erased representation** (db.rs has a whole fixpoint pass, PR#659, keeping embedded `Ty::Sum`
back-edges normalized to the one-level `Ty::Nominal{inner}` shape so wasm's `valtype_of`/
`is_heap_type` and the rust `rust_type` never disagree). wasm RELIES on a recursive newtype being
erased (heap handle). So the fix must NOT remove the recursive newtype from `newtype_inner`, and must
NOT change `infer::newtype_underlying`. It has to be a **Rust-backend-local** decision:

> A recursive newtype (a `Ty::Nominal` whose `inner` transitively mentions its own `decl`) is
> un-erased ON THE RUST BACKEND ONLY: it emits a real Box-indirected nominal type and its
> construct/match/projection go through that type, while wasm keeps erasing it.

## Proposed increments (each independently gate-able)

**Detector (shared helper, no behavior change).** A `nominal_is_recursive(db, decl)` predicate:
`inner` (from `newtype_inner`, or re-derived) transitively reaches `decl` via the sum-reference
graph — reuse `reaches_decl`/`mentions_decl` already in enums.rs. This is the single gate every
increment below keys on. Land it with a unit test first (pure addition).

**B1 — type emission.** In `types::rust_type`, add a `Ty::Nominal` arm BEFORE the erase-through
(types.rs:112): when `nominal_is_recursive`, render the nominal's own sanitized name (a real Rust
type `Lst`), not `rust_type(inner)`. In `emit_one_enum`, replace the blanket newtype decline with:
if `nominal_is_recursive`, DON'T skip — emit a single-variant type for it. Shape options:
  - a one-variant `enum Lst { Mk(::std::boxed::Box<Inner>) }` — reuses the sum path verbatim (the
    recursive-variant box + derives), lowest-risk; OR
  - a newtype `struct Lst(::std::boxed::Box<Inner>)` — closer to the "single ctor" intent.
  Recommend the **enum** form: it lets the existing `SumNew`/`SumPayload` construct/match paths work
  unchanged (a newtype's `Mk` is disc 0), where a struct would need its own construct/project arms.
  The box wraps the recursive `inner` exactly as the recursive-sum variant does.

**B2 — construction.** `(Mk (Some …))` builds `Lst::Mk(Box::new(<inner>))`. If B1 uses the enum
form, `sum_variant_path_of_ty` + the `SumNew` emit already produce `Lst::Mk(…)`; only the box wrap
needs to fire, which `variant_is_recursive` already drives for sums. Verify the erased-newtype
construct path (which today builds the bare inner) is correctly superseded for the recursive case.

**B3 — projection / match.** `(match l ((Mk o) …))` and `(. p 1)` (projecting the recursive `Lst`
field out of the payload tuple). The match unwraps `Lst::Mk(__pay)` and derefs the box; the tuple
projection reads the `Lst`-typed slot. The corpus doc for the traversal case notes the μ back-edge
(`Ty::Sum{decl}`) vs the folded `Ty::Nominal{decl}` "must unify" — confirm the rust path types the
projected field as the nominal so the recursive `sm` call type-checks.

**B4 — equality / ordering / collection key** (only if a case needs it). `Box<T: Eq>: Eq`, so the
derives compose; a recursive newtype as a map key would ride the same path as a recursive sum.
Defer until a corpus case exercises it (neither current case does).

## Risks & things to verify

- **wasm untouched.** Assert the wasm gate is byte-identical after each increment (the change is
  gated on `nominal_is_recursive` inside `backend/rust/`, and `newtype_inner`/`infer` are not
  touched). This is the load-bearing invariant — any drift means the erasure contract broke.
- **Non-recursive newtypes stay erased.** The detector must be exact: a plain `(type UserId (Mk
  Int64))` keeps erasing to `i64`. Only a self-mentioning inner un-erases.
- **Mutual recursion.** `(type A (AN B))` + `(type B (BN A))` newtypes — `reaches_decl` already
  catches the mutual cycle for sums; the detector should too. Add a witness.
- **Generic recursive newtype.** `(type Box (W a) …)` style — out of scope for the first landing;
  the current cases are monomorphic. Note the deferral in the emit so it declines cleanly.
- **The gate-baseline flip** is 2 cases × 2 rust baselines (`.gate-baseline-rust` +
  `-rust-async`) — targeted one-line edits, single commit, NOT `gate --save`.

## Non-goals

- Changing `infer::newtype_underlying` or `db.newtype_inner` (breaks wasm; ruled out).
- A named Rust struct for NON-recursive newtypes (they erase correctly; a struct is future polish
  tracked elsewhere, not this gap).
- The mutual-recursion `(List Any)` decoder case (inference lane, not rust-emit).
