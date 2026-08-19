# DESIGN: recursive-newtype un-erasure (Box-indirected nominal emission) on the Rust backend

Status: **RESOLVED — option-2 (accept the clean decline as a permanent floor)** (2026-08-19). Owner:
`v-rust-backend`. The concierge greenlit the slice in principle (answer, greenlight-default), but the
B1 prototype then hit the shared-lowering spec-MUST-erasure BLOCKER below: the `Mk` node is erased in
`lower.rs` (cited to a `type-system.md` MUST) before any backend sees the core, so the ONLY fix is
option-1 (the Rust backend re-synthesizes the box at the nominal boundary) — invasive, fights the
spec-MUST the whole nominal model rests on, and high wasm-drift risk. Per the concierge's OWN guardrail
("if a step's regression surface looks bigger than expected mid-slice, PAUSE + re-scope rather than
force it"), and given the shape is rare (2 corpus cases) with an already-precise decline diagnostic
(`fc4eb13a1`), the resolution is **option-2: keep the honest clean decline; wasm remains the backend
for recursive-newtype programs.** This is a spec-mandated parity floor, NOT a bug. Reopen ONLY if a
non-niche consumer appears AND the erasure model changes to leave a nominal node for the backend.

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

## ⛔ BLOCKER (2026-07-30, second prototype): the erasure is in SHARED LOWERING, cited to a spec MUST

The "rust-backend-local un-erasure" premise above is **WRONG** — re-examining the construct/match
path proved the newtype tag is erased in **shared lowering** (`lower.rs`), NOT in the backend:

- A `(Mk n)` destructure lowers its `Payload` step away via `erase_nominal_steps` (lower.rs:388): the
  step "is a runtime no-op (the box is erased), so it emits no `sum-payload` — DROP it from the path
  the backend walks. … an empty path reads the scrutinee value directly (`(Mk n)` binds `n` to the
  whole erased value)." The construct side likewise lowers `(Mk (Some …))` to just the inner
  `(Some …)` — no `Core::SumNew` for the `Mk` tag.
- This is cited to a spec **MUST**:
  `spec/capabilities/type-system.md#a-nominal-value-is-convertible-to-its-underlying-structural-value`
  — "stripping a nominal's name tag … MUST be a compile-time reinterpretation and not a copy or
  conversion", and "the stripped structural value MUST be the same value the nominal already is at
  runtime." (Same full path lower.rs:384/386's duvet citation uses.)

So by the time ANY backend sees the core, the `Mk` construct/match/payload have already vanished.
The B1 prototype confirmed this: with the type un-erased, construct emitted the bare inner
`sm(Option::Some(…))` (not `sm(Lst::Mk(Box::new(…)))`) and match read `Option::Some(pay)` against an
`Lst`-typed scrutinee → E0308. The backend has no `Mk` node left to route through a box.

**Consequence — there is NO clean backend-local fix.** Two real options, each with a cost:

1. **Rust backend RE-SYNTHESIZES the box at the nominal boundary.** Every `SumPayload` (empty path
   over a recursive nominal) wraps a deref, every value of recursive-nominal type at a
   construct/return/arg boundary wraps `Lst::Mk(Box::new(..))`, and the type flows so rust and the
   erased core agree. This is invasive (touches the shared `SumPayload`/`SumNew`/projection emit with
   nominal-awareness the lowering deliberately removed) and fights the "backend needs no nominal
   awareness" design of `erase_nominal_steps`. High risk of wasm drift and subtle boundary bugs.
2. **Accept the rust decline as a permanent floor.** A recursive newtype is a rare shape (two corpus
   cases); the diagnostic already names the gap precisely (`fc4eb13a1`). Rust stays a strict subset
   here; wasm remains the backend for recursive-newtype programs. Zero risk, zero code.

The earlier increment plan (B1→B4, "one coherent slice") is **not viable as written** — it assumed
the backend still had the `Mk` node. Do NOT implement it.

**DECISION (2026-08-19): option-2 — keep the clean decline as a permanent floor.** (See the Status
header.) The erasure is load-bearing and spec-mandated (`type-system.md` MUST), the shape is rare (2
corpus cases), the decline is already honest (`fc4eb13a1` names the precise reason), and option-1's
risk/reward is poor (invasive nominal-aware re-synthesis fighting a spec-MUST, high wasm-drift risk) —
which the concierge's own "pause + re-scope if the regression surface is bigger than expected"
guardrail directs away from forcing. Recursive newtype is therefore a **spec-mandated rust-vs-wasm
parity floor**, not a fixable gap. The rust-coverage mission is at its practical ceiling: this + the
first-class-type feature + the adapter-records case are the residual non-gaps.

## B1 PROTOTYPE FINDINGS (2026-07-30, validated then reverted)

A throwaway prototype of the detector + `rust_type` naming + `emit_one_enum` un-skip +
`sum_representable` flip was built and probed against the `Lst` corpus case, then reverted (the tree
stays clean until the full slice lands). What it PROVED:

- **The type emission is exactly right.** With the detector wired, the backend emits
  `pub enum Lst { Mk(::std::boxed::Box<Option<(i64, Lst)>>) }` — the intended Box-indirected
  one-variant enum, derives and all. So B1's `rust_type`-names-the-nominal +
  `emit_one_enum`-emits-the-boxed-enum is sound and small.
  - `rust_type` detects it **db-FREE**: `mentions_decl(inner, *decl)` (made `pub(super)`) walks
    `inner` for the self-reference, so the `Ty::Nominal` arm stays a pure `Ty`→`String` map. Add the
    recursive arm BEFORE the erase-through arm.
  - `emit_one_enum`: change the newtype skip to
    `if db.newtype_inner.contains_key(&decl.occ) && !nominal_is_recursive(db, decl.occ)` — a recursive
    newtype falls through to the ordinary enum emission, where `variant_payloads_mention` already
    drives the `::std::boxed::Box<…>` boxing of the recursive `Mk` payload.
  - `sum_representable`'s `Ty::Nominal` arm must stop declining a recursive newtype: when
    `mentions_decl(inner, decl)`, return `nominal_is_recursive(db, decl) && args…representable` and
    do NOT recurse into `inner` (the μ back-edge loops; the box makes it finite, as for a sum).

- **B2/B3 is the real remaining work — construct + match still ERASE the newtype.** With B1 alone
  the case now fails to COMPILE (E0308) instead of declining, because the construct/match sites still
  treat `Lst` as erased:
  - construct emits `sm(Option::Some(( … )))` — the bare inner `Option<(i64, Lst)>`, NOT
    `sm(Lst::Mk(::std::boxed::Box::new(Option::Some(…))))`. It must wrap the `Mk` ctor + box.
  - match emits `match __ms { Option::Some(__pay) => … }` against an `Lst`-typed scrutinee — it must
    first unwrap `Lst::Mk(__pay)` and deref the box, THEN match the inner `Option`.
  These sites currently consult `newtype_inner` to SKIP the `Mk` tag (the erasure). For a recursive
  newtype they must NOT skip — they must go through the same `SumNew`(box)/`SumPayload`(deref) path a
  recursive sum uses. That is the crux of B2/B3 and why B1 cannot land alone (it would regress the
  clean decline into an uncompilable emit).

- **CONCLUSION: land B1+B2+B3 as ONE coherent slice**, not B1 alone. The slice = detector +
  `rust_type` arm + `emit_one_enum` un-skip + `sum_representable` flip + the construct/match
  un-erasure for a recursive newtype (route through the sum box/deref instead of the erasure skip).
  Then the 2-case×2-baseline `todo`→`pass` flip. The construct/match un-erasure is the part that
  needs care: find where `SumNew`/`SumPayload`/the projection consult `newtype_inner` and gate the
  skip on `!nominal_is_recursive`.

## Proposed increments (land as ONE slice — see B1 findings above)

The prototype showed B1 cannot land alone (it regresses the clean decline into an E0308 emit). The
four faces below are ONE merge-request; each bullet is a hunk of it, not a separate landing.

**Detector (shared helper).** A `nominal_is_recursive(db, decl_occ)` predicate = `decl_occ` is in
`db.newtype_inner` (an erasable newtype) AND its sole variant's payload reaches the decl
(`variant_payloads_mention(db, &variants[0], decl)` → `reaches_decl`, already in enums.rs). Place it
`pub(super)` next to `variant_is_recursive`. Confirmed exact: a scalar newtype (`(type UserId (Mk
Int64))`) has no reach so stays erased; a multi-variant recursive SUM is not in `newtype_inner` so is
untouched (its enum path already boxes); a mutual-recursion newtype cycle is caught by `reaches_decl`.
It is dead code until the emission hunks below use it — so it lands WITH them, not before.

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
