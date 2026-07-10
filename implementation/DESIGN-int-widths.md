# Design — width-indexed integers `(Int N)` / `(UInt N)` in `rcdzc`

**Author:** compiler engineer. **Audience:** whoever grows `rcdzc` next (task #152), and future me.
**Status:** **DESIGN ONLY — nothing landed.** Line numbers are landmarks at this commit (2026-07-09,
tree at 7aa0cc5), not promises they won't drift. `rcdzc` today lowers *every* integer as an i64 with a
single `Ty::Int` leaf; there is no width machinery. This doc designs the whole width family against the
real structs.

> **Not normative.** The *what* is fixed in
> [numeric-model.md](spec/capabilities/numeric-model.md) (capability) and pinned concretely in
> [options/numeric-model/explicit-checked.md](options/numeric-model/explicit-checked.md) (the widths,
> representations, boundary mapping). The behavioral contract is **already pinned green-when-realized** in
> the corpus at [spec/semantics/06-numeric-model.sexp:950–1265](spec/semantics/06-numeric-model.sexp) —
> ~40 cases, all `(needs numeric-model)`, skipped today. This is the *how* against `rcdzc`'s seams.

This continues the first-class-types chain: L1 (type-values), L1.5 (compile-time closures), **L2**
(parametric type-ctors as type-builder **intrinsics**, commit 7aa0cc5). Width-indexed integers are the
**litmus** for that chain, and it is the operator's chosen next increment.

> **⚡ ADVERSARIAL REVIEW APPLIED (2026-07-09).** Two independent source-grounded reviewers attacked an
> earlier draft; every finding below is folded into the relevant section, and the load-bearing ones are
> collected in **§0**. Confirmed sound by review: the 36-`Ty::Int`-site count (verified exactly 36),
> `build_int_ty` as the single width validator, alias equivalence via an identical `Ty`, no-promotion via
> the one `unify` arm, all four CDZ0302 cases, per-width const-fold→CDZ0304, and the clean BigInt/Rational/
> Float *type* deferral. The corrections that changed the design are in §0.

---

## 0. Corrections from adversarial review (read before implementing)

These are defects found against live source in an earlier draft; the sections that follow already
incorporate the fixes. Ranked by severity.

1. **[CRITICAL — the governance flip cannot select widths alone; §12].** The corpus skip is keyed on ONE
   coarse tag: `corpus.rs:72–73` skips a case iff any `(needs X)` is unrealized, and **all 49**
   numeric-model cases carry the *same* `(needs numeric-model)` — 27 width cases PLUS 22 non-width
   (9 rational, 5 BigInt, 5 default-integer pragma, 2 float, 1 wrapping-type). There is no
   `integer-widths` tag. So "declare widths realized" by adding `"numeric-model"` to `REALIZED`
   un-skips ALL 49; the 22 unimplemented ones then RUN and FAIL (mostly CDZ0101 unbound-name —
   `Rational.of`/`BigInt.of`/`Wrapping64` — on a runnable-primary case, `corpus.rs:420–430`). **The
   governance act MUST also re-tag the 27 width cases (`06-numeric-model.sexp:971–1265`) to a NEW finer
   capability (e.g. `integer-widths`) and realize only that.** Corrected in §12. (The narrower claim — the
   *code* behind the skip leaves the FAIL set unchanged *before* any flip — does hold: with
   `numeric-model` unrealized, all 49 stay skipped.)

2. **[HIGH — `(UInt 48)` terminal outputs are undeliverable as designed; §9/§10].** Cases `:1186`
   (`(: 2⁴⁸−1 (UInt 48))` as the program's output) and `:1206` (`(UInt 48).wrap …` → a `(UInt 48)` output)
   make a *non-aliased-width value the observed terminal result*. But a non-aliased width has
   `comp_valtype = None`, and a pure-scalar body takes the SCALAR component path
   (`layout.rs` `imports_runtime` is false for a plain integer; the entry is lifted through
   `Ty::comp_valtype`, `serialize.rs:120–137`) → the component can't be built → **uncoded decline →
   scored TODO, never Pass** (`corpus.rs:431`). The design must route a width-typed terminal return
   through the **render-to-string** path (force `imports_runtime`/`body_uses_heap` for a non-aliased-width
   — arguably any observed-width — entry return), the same way a compound result already renders in-program.
   This is also the answer to the spec MUST "what happens if you export a `(UInt 48)`" (it does not cross
   as a primitive; it renders). Corrected in §9/§10.

3. **[HIGH — shift-count typing contradicts the corpus; §7].** The earlier draft typed a shift COUNT as
   rigid `Int64`. But `:1130` writes `(>> UInt8.max (: 1 UInt8))` — the count is `(: 1 UInt8)`. Typing the
   count `Int64` rejects the corpus input. **Fix: the count may be any integer width; the value operand's
   width is what the result takes and what the 0..N count guard is checked against.** Corrected in §7.

4. **[MEDIUM — CDZ0301 must not swallow CDZ0201; §7/§11].** "Any operand `unify` failure → CDZ0301" would
   regress a *non-numeric* operand (`(+ 2 true)`) from CDZ0201 to CDZ0301 (today `infer.rs` arith →
   `Code::TypeError`). **CDZ0301 fires only when both operands are numeric-but-distinct; a non-numeric
   operand stays CDZ0201.** Latent (no live case exercises it) but a real schema violation. Corrected in §7.

5. **[MEDIUM — annotation-time width evaluation is not available; §7].** Infer runs BEFORE lower/fold
   (`pipeline.rs:32` infer, then lower, then fold), so there is **no Mir evaluator callable at annotation
   time**. `extract_type_value` (infer.rs:1107) reduces `(List Int64)` only because `Int64`'s arg is a
   `TypeVal` *leaf*. So a width arg must be a **literal `Int`** in Stage 1+2; `(UInt (+ 4 4))` is DEFERRED
   (not "locally folded" — that was a hand-wave). Corrected in §7 and §14 decision 2.

6. **[MEDIUM — the `.max`/`.of` member dispatch has a resolve-seam dependency; §8].** `(. UInt8 max)`
   reaches infer's `RecordProj` arm as a projection on a `TypeVal` ONLY because `member()`'s
   `Some(_) => {}` arm (`resolve.rs:1010`) falls through to `RecordProj` for a non-record prelude entry —
   which holds because aliases resolve to **bare `TypeVal`s, not module records**. `(. (UInt 48) max)`
   bypasses the `Node::Name` block entirely and reaches `RecordProj` directly. Both land in the same infer
   arm, but this is a correctness *dependency* (aliases must be TypeVals) the design now states explicitly
   in §6/§8, per the L2 "member access is a separate path" warning.

7. **[LOW — runtime-width rejection has an ordering hazard; §7].** `(: 5 (UInt n))` → CDZ0302 is correct
   ONLY if the width check fires while inferring the *definition body* (where `n` is an opaque param),
   before constant-argument inlining could launder `(mk 8)`'s `n` to `8` (the corpus's own runtime-value
   note, `:408–416`, warns of exactly this laundering). Stated as a hazard in §7.

**Non-issue (operator ruling):** `Intrinsic` need **not** stay `Copy` — a payload-carrying `IntOf(IntTy)`/
`IntWrap(IntTy)` variant is fine regardless. (It happens to already carry `Heap(HeapIntrinsic)` and stay
Copy, but Copy is not a constraint on this design.)

---

## 1. Thesis (one paragraph)

A width-indexed integer type is **L2 with the type-constructor's argument shifted from a *type* to a
*natural***, and its arithmetic is **the checked-i64 core the seed already has, generalized to a width
computed from N**. `Int` and `UInt` become two more type-builder **intrinsics** — of arity `Nat → Type`
instead of `Type → Type` — riding the *same* `Apply(Intrinsic)` const-fold path that `List`/`Map`/… ride
since L2. `Int64` is the alias `(Int 64)`. The whole point of indexing the width (rather than shipping
eight primitives) is that an unusual-but-useful width — a `(UInt 48)` timestamp, a `(UInt 62)` tagged
pointer — is a **first-class type the compiler *computes*** (its mask, bounds, and ops all follow from N),
not a wrapper the author hand-writes. This is exactly the payoff first-class types promise, and the
general form is *simpler* to implement than eight special cases because the arithmetic is already
width-parametric.

**Why it is the litmus.** It is the first construct where (a) a **compile-time value** (a natural)
parameterizes a *type*, and (b) the type's **runtime lowering is computed from the type parameter** — the
overflow bounds, the low-N mask, and the signed-vs-unsigned machine op (`lt_s`/`lt_u`, `shr_s`/`shr_u`,
`div_s`/`div_u`) all follow from N. First-class types lowering to the right intrinsics, end to end.

## 2. Scope (operator's decisions, 2026-07-09)

- **Representation: single canonical `Ty::Integer(IntTy)`** — *delete* `Ty::Int`. One representation of
  an integer type, `Int64` == `Integer{signed:true,width:64}`. (The additive `Ty::IntN`-beside-`Ty::Int`
  option was considered and rejected: two spellings of one type is the coarse-Kind footgun the rewrite
  exists to kill. The cost is a one-step rewrite of all 36 `Ty::Int` sites; §4 makes it mechanical.)
- **Land Stage 1 + Stage 2 together** — construction/overflow/no-promotion/constraint/alias/op-selection
  **and** the `.max`/`.min`/`.of`/`.wrap` bounds & conversions.
- **Planning only for now.** This doc is the plan; implementation is a separate act.
- **Governance flip is a separate, later act** (§12): the corpus cases stay skipped until the realized
  set declares widths realized, **the 27 width cases are re-tagged to a new `integer-widths` capability**
  (§0 finding 1 — the existing `numeric-model` tag is too coarse to select widths alone), and the seven
  boundary rows land in the frozen `component-abi.md`.

## 3. The acceptance target (what the corpus pins)

`06-numeric-model.sexp:950–1265`. Grouped:

| Group | Representative cases | Obligation |
|---|---|---|
| Construction / bounds | `(: 200 UInt8)`→200:UInt8; `UInt8.max`=255; `Int8.min`=−128; `UInt64.max`=2⁶⁴−1; `(: 2⁴⁸−1 (UInt 48))` | a non-64 width is reachable; each has its own bounds |
| Per-width overflow | `(+ (: 255 UInt8)(: 1 UInt8))`; `(- (: 0 UInt8)(: 1 UInt8))`; `(+ (: 127 Int8)(: 1 Int8))`; `(+ UInt32.max (: 1 UInt32))`; `(+ (: 2⁴⁸−1 (UInt 48))(: 1 (UInt 48)))` | checked at **its own** range → `(trap "integer overflow")` |
| No promotion | `(+ (: 1 UInt8)(: 2 Int32))`; `(+ (: 1 Int32)(: 2 UInt32))` | different width OR signedness → **CDZ0301** |
| Conversions | `(UInt8.of (: 200 Int32))`=200; `(UInt8.of (: 256 Int32))`→trap; `(UInt8.of (: -1 Int32))`→trap; `(UInt8.wrap (: 256 Int32))`=0; `(UInt8.wrap (: -1 Int32))`=255; `((UInt 48).wrap (: -1 Int64))`=2⁴⁸−1 | `T.of` checked (trap out of range); `T.wrap` keeps low N bits |
| Signedness selects op | `(< (: 0 UInt64) UInt64.max)`=true; `(>> UInt8.max (: 1 UInt8))`=127 | unsigned → `lt_u`/`shr_u`; signed → `lt_s`/`shr_s` |
| Alias equivalence | `(: 200 (UInt 8))` ≡ `(: 200 UInt8)`; `(: (: 5 (UInt 32)) UInt32)` NOT CDZ0203 | `UInt8` and `(UInt 8)` are the **same** `Ty` |
| Width constraint | `(UInt 0)`, `(UInt 65)`, `(UInt 128)`, `(UInt n)` with runtime `n` | out-of-range / non-constant → **CDZ0302** |

Concrete pins (`explicit-checked.md`): `N ∈ 1..=64`; `Int8/16/32/64`+`UInt8/16/32/64` are aliases
(`(def UInt8 (UInt 8))`), not primitives; only those eight cross the boundary (`(Int 8)→s8 … (UInt 64)→u64`),
a non-aliased width is internal-only; representation computed from N (mask low N unsigned / sign-extend
from bit N−1 signed); overflow compares against `±2ⁿ⁻¹` / `2ⁿ`.

## 4. `ty.rs` — the representation and the mechanical rewrite

```rust
/// A fixed-width integer descriptor. INVARIANT: width ∈ 1..=64. (`Copy` is convenient but NOT required —
/// per operator ruling, `Intrinsic` need not stay `Copy`, so `IntOf(IntTy)`/`IntWrap(IntTy)` are fine
/// whether or not this derives Copy; keep it Copy anyway since the fields are trivially so.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntTy { pub signed: bool, pub width: u8 }

impl IntTy {
    pub const I64: IntTy = IntTy { signed: true, width: 64 };   // == the old Ty::Int
    pub fn min(self) -> i128 { if self.signed { -(1i128 << (self.width-1)) } else { 0 } }
    pub fn max(self) -> i128 { if self.signed { (1i128 << (self.width-1)) - 1 } else { (1i128 << self.width) - 1 } }
    /// Fold a value INTO this type's range (two's-complement low-N bits), returning the canonical i128.
    pub fn wrap(self, v: i128) -> i128 { /* mask to N bits; sign-extend from bit N-1 if signed */ }
    pub fn fits(self, v: i128) -> bool { (self.min()..=self.max()).contains(&v) }
    /// The eight aliased widths cross the boundary; others are internal-only.
    pub fn comp_valtype(self) -> Option<u8> { /* (s,8)->s8 … (u,64)->u64 ; else None */ }
    /// Alias name for rendering (`"UInt8"`); None → render the width-indexed form `(UInt N)`.
    pub fn alias_name(self) -> Option<String> { /* Some for the 8 aliased widths */ }
}

pub enum Ty {
    Integer(IntTy),   // REPLACES Ty::Int. Ty::Integer(IntTy::I64) is Int64.
    Bool, Unit, /* …unchanged… */
}
```

**The mechanical rewrite (all 36 `Ty::Int` sites).** `grep -n 'Ty::Int\b'` → 36 hits across 7 files. Each
is one of two shapes:
- A **construction** `Ty::Int` (a literal's type, an intrinsic signature's `Ty::Int` operand/result, a
  scratch `Ty::Int`) → `Ty::Integer(IntTy::I64)`. Byte-identical behavior for the Int64 path — this is the
  regression surface to pin (§10).
- A **match arm** `Ty::Int =>` → `Ty::Integer(it) =>` and then branch on `it` where width matters:
  - `core_valtype` (ty.rs:314): `Ty::Integer(_) => Some(ValType::I64)` — **all widths i64 in Stage 1**
    (§7 defers i32 packing). Uniform, so this is a *simplification* of the existing arm, not a split.
  - `comp_valtype` (ty.rs:337): `Ty::Integer(it) => it.comp_valtype()` — `IntTy::I64`→`s64` (unchanged);
    the eight aliased widths → their primitive; a non-aliased width → `None` (internal-only).
  - `is_comptime_only`/`occurs`/`subst_params`/`is_compound`: `Ty::Integer(_)` is an ordinary scalar leaf
    (false / clone), same as `Ty::Int` was.
  - `unify` (ty.rs:458): replace the `(Ty::Int, Ty::Int)` ground arm with
    `(Ty::Integer(x), Ty::Integer(y)) if x == y => Ok(())`. **Two integer types unify iff same signedness
    AND width** — this single arm *is* no-promotion at the unification layer.
  - `render` (render.rs:112,156): `Ty::Integer(it) =>` itoa the magnitude; for an unsigned value read the
    i64 bit-pattern as u64 (so `UInt64.max` prints `18446744073709551615`). ⚠ see §9.

## 5. The intrinsics — `TypeInt` / `TypeUInt` (`Nat → Type`)

Two `Intrinsic` variants wired **exactly** like L2's `TypeList`… — the *only* difference is the argument
type in `signature()` is an integer (a natural), not `Ty::Type`.

```rust
// ir.rs enum Intrinsic { …, TypeInt, TypeUInt }
// param_count(): 0 (monomorphic — the width is a VALUE arg, not a Ty::Param).
// signature():  TypeInt|TypeUInt => (vec![Ty::Integer(IntTy::I64)], Ty::Type)   // Nat → Type
```

`fold_const` — the L2 pattern, reading an `Int` arg, routing a bad width through the **existing poison
machinery** (a width-constraint violation is a compile-time-provable ill-formedness, exactly like the
CDZ0304 const-trap poison it sits beside):

```rust
Intrinsic::TypeInt | Intrinsic::TypeUInt => {
    let signed = matches!(self, Intrinsic::TypeInt);
    match args {
        [Mir::Int(n)] => Some(build_int_ty(signed, *n as i128)      // shared with infer, §6
            .map(Mir::TypeVal).unwrap_or_else(Mir::Error)),         // CDZ0302 → poison
        _ => None,   // arg not a folded constant → stays residual; infer already rejected a runtime width
    }
}
```

```rust
/// The SOLE constructor of an integer Ty from (signedness, width). Enforces 1..=64 (CDZ0302) in ONE
/// place; shared by fold_const AND infer::extract_type_value so the two paths cannot drift (the L2 lesson).
pub fn build_int_ty(signed: bool, width: i128) -> Result<Ty, Reject> {
    if !(1..=64).contains(&width) {
        return Err(Reject::coded(Code::WidthConstraint,
            format!("integer width {width} is outside the admitted range 1..=64")));
    }
    Ok(Ty::Integer(IntTy { signed, width: width as u8 }))
}
```

Add `TypeInt`/`TypeUInt` to `param_count`'s 0-arm and to every inert-leaf / `is_transient` /
`alpha_rename` / `substitute` / `collect_calls` list an intrinsic already appears in — they are ordinary
`Intrinsic`s, so **zero new IR node variants** (same as L2; `grep TypeCtor` stayed 0 there, `grep` for a
new node here stays 0 too).

## 6. `resolve.rs` — bare names, aliases, and the literal question

**Bare `Int`/`UInt`** → their builder intrinsic, in the special-case block (resolve.rs:716–728), parallel
to `List`→`TypeList`:
```rust
"Int"  => Hir::Intrinsic(Intrinsic::TypeInt),
"UInt" => Hir::Intrinsic(Intrinsic::TypeUInt),
```
So `(Int 64)` folds to `TypeVal(Ty::Integer(I64))` — identical to `Int64`; `(UInt 8)` to
`TypeVal(Ty::Integer{false,8})`.

**The 16 aliases** `Int8..Int64`, `UInt8..UInt64` → prelude **bare `TypeVal`s** built from `build_int_ty`
in a loop in `prelude::build` (not 16 hand entries), so an alias and its expansion are the *same* `Ty`
(alias equivalence for free — `(: (: 5 (UInt 32)) UInt32)` is not a CDZ0203 conflict because both extract
the identical `Ty::Integer{false,32}`). These are **dual-role** like `Int64`: bare `UInt8` → the
type-value; `(. UInt8 max)` → the per-width op (§8).

> **⚠ Load-bearing choice (review finding 6): each alias is a bare `TypeVal`, NOT a module record** — the
> opposite of the current `Int64` prelude entry (a `Hir::Record` of ops, `prelude.rs:124`). This matters
> because `(. UInt8 max)` must reach infer's `RecordProj` arm as a projection on a type-value (§8), and
> `member()` (`resolve.rs:964`) only falls through to `RecordProj` for a *non-record* prelude entry (the
> `Some(_) => {}` arm at `resolve.rs:1010`); if `UInt8` were a record, `member()` would try to project a
> field named `max` OFF that record and decline instead. So the width family deliberately does NOT model
> `UInt8` as an ops-record — the ops are computed from the type-value at the projection, not stored as
> record fields. (`Int64` today is a record only because its ops predate the width machinery; migrating it
> to the type-value dispatch is a consistency follow-up, not required here — but do NOT add a `UInt8`
> record.) `(. (UInt 48) max)` has an `Apply` operand, bypasses the `Node::Name` block in `member()`
> entirely, and reaches `RecordProj` directly — so both aliased and width-indexed forms land in the one
> infer arm.

**⚠ THE LITERAL-TYPING FORK (the sharpest design decision — review this).** Today `Hir::Int(n)` types
rigidly as `Ty::Int`. Under the single representation, `(: 200 UInt8)` must be *well-typed* (200:UInt8),
so a bare literal **cannot** be rigidly Int64 — unifying a rigid-int64 literal against UInt8 would wrongly
be CDZ0203. Two models:

- **Model L (literal adopts at the annotation) — RECOMMENDED.** A bare `Hir::Int(n)` stays
  `Ty::Integer(IntTy::I64)`. The **annotation arm** (infer.rs:578) special-cases: when `e` reduces to an
  integer *constant* and the annotation `T` is an integer type and the constant *fits* `T` (`IntTy::fits`),
  the annotation **succeeds** and the result type is `T` (the literal adopts the width); otherwise ordinary
  `unify` (a width/sign mismatch on a non-literal → CDZ0203). No new type-var machinery. This matches the
  corpus exactly: every non-64 width is reached through an explicit annotation, a per-width bound, or a
  conversion — never through an unannotated bare literal.
- **Model V (integer-kinded literal var).** `Hir::Int(n)` → a fresh var *constrained to integer kind*,
  defaulted to Int64 at finalize if unsolved. More principled (textbook HM-with-defaulting) but requires
  a *kind* on type-vars (the seed HM has none) to stop a literal var binding to `Bool`; that is real new
  machinery the seed deliberately lacks. **Rejected for now** as over-engineering the corpus doesn't force.

**The default-integer rule still needs a small defaulting step**, independent of the fork above: a
genuinely unconstrained integer *param* var — `(def (add a b) (+ a b))`, where neither operand is a
literal — must resolve to Int64. `require_integer` (§7) handles this: an unsolved var in an integer
operand position unifies with `Ty::Integer(IntTy::I64)`.

## 7. `infer.rs` — operand typing, no-promotion, annotation extract

**Arith/Bit/Shift/Cmp operands (infer.rs:495–545).** Today arith hardcodes both operands to `Ty::Int`
(`unify_at(&ta.ty,&Ty::Int,…)`). Generalize to "both operands the SAME integer type; a mismatch is
no-promotion":
```rust
Hir::Arith(op, a, b) => {
    let ta = self.expr(a)?; let tb = self.expr(b)?;
    // BOTH operands must first BE integers (a non-numeric operand is ordinary ill-typedness, CDZ0201 —
    // NOT no-promotion). require_integer enforces that AND defaults an unsolved var to I64.
    let ita = self.require_integer(&ta.ty)?;   // Integer(it) | default I64 ; else CDZ0201
    let itb = self.require_integer(&tb.ty)?;
    // Two integers that DIFFER in width/signedness are the no-promotion case (CDZ0301), not CDZ0201.
    if ita != itb {
        return Err(Reject::coded(Code::NoPromotion,               // CDZ0301
            "an operation on two different numeric types requires an explicit conversion"));
    }
    Ok(Typed { node: TypedNode::Arith(*op, ita, Box::new(ta), Box::new(tb)),  // ← node CARRIES the IntTy
               ty: Ty::Integer(ita) })
}
```
`require_integer(ty)` (review finding 4 — code discipline): `apply` the subst; `Ty::Integer(it)` → `it`;
an unsolved `Ty::Var` → unify it with `Ty::Integer(IntTy::I64)` and return `I64` (the default-integer
rule); **anything else (Bool, a compound, …) → CDZ0201** (`Code::TypeError` — a non-numeric operand is
general ill-typedness, exactly what the seed does today; do NOT emit CDZ0301 here). CDZ0301 is reserved
for the "two *different numeric* types" case — `ita != itb`. This keeps `(+ 2 3)`→Int64, `(+ a b)`→Int64,
`(+ 2 true)`→CDZ0201 (no regression), makes `(+ (:255 UInt8)(:1 UInt8))`→UInt8, and
`(+ (:1 UInt8)(:2 Int32))`→CDZ0301. **The node now carries the operand `IntTy`** — this threads the width
to lowering (§9). `Cmp` already has an `operand_ty` field (ir.rs:759); apply the same rule, keep result
`Ty::Bool`.

**Shift (review finding 3 — the count is not rigidly Int64).** The corpus writes
`(>> UInt8.max (: 1 UInt8))` (`:1130`) — the COUNT operand is `(: 1 UInt8)`, a UInt8. So the count must
accept **any integer width** (`require_integer` on it, ignore its specific width), not be unified against
Int64 (which would reject the corpus input). The RESULT type and signed-vs-logical shift selection come
from the **VALUE** operand's `IntTy`; the 0..N count-range guard is checked against the **value's** width N
regardless of the count operand's own type.

**Annotation extract (infer.rs:601, `extract_type_value` at 1107).** Extend the existing `Apply`-of-a-
type-builder-intrinsic arm (which today reduces `(List Int64)` because `Int64`'s arg is a `TypeVal` *leaf*)
to also handle `Apply(Intrinsic(TypeInt|TypeUInt),[arg])`:
- arg is `TypedNode::Int(n)` → `build_int_ty(signed,n)`; `Err` propagates as the annotation's reject
  (CDZ0302 — covers `(: 5 (UInt 65))`, `(: 0 (UInt 0))`, `(: 5 (UInt 128))`).
- arg is anything else (a `Local`, an arithmetic expr, …) → **CDZ0302** "an integer width must be a
  compile-time constant" (covers the runtime-param `(def (mk n) (: 5 (UInt n)))` — the point that keeps
  the feature indexed-not-dependent).

> **⚠ Review finding 5 — annotation-time width evaluation is NOT available; a width must be a literal in
> Stage 1+2.** The earlier draft said to "locally fold" a width expression like `(UInt (+ 4 4))`. That was
> a hand-wave: the pipeline runs **infer, THEN lower, THEN fold** (`pipeline.rs:32`), so at annotation time
> — inside `extract_type_value`, during inference — there is **no Mir evaluator to call**. `extract_type_value`
> works for L2 only because it recurses on `TypeVal` leaves, never evaluating anything. So: **require the
> width arg to be a literal `TypedNode::Int`**; `(UInt (+ 4 4))` is DEFERRED (it declines/CDZ0302 as
> "non-constant" for now — no corpus case needs a computed width). Lifting this later means either (a) a
> tiny closed-integer const-evaluator callable from infer, or (b) moving annotation extraction to run after
> a pre-fold. Not Stage 1+2.

> **⚠ Review finding 7 — the runtime-width rejection has an ordering hazard.** `(: 5 (UInt n))` → CDZ0302
> is correct because, while inferring the *body of `mk`*, `n` is an opaque `Local` (not an `Int`), so the
> "arg is not a literal" branch fires. This depends on the check running at the DEFINITION site before any
> constant-argument inlining. The corpus's own runtime-value note (`06-numeric-model.sexp:408–416`) warns
> that inlining `(mk 8)` would launder `n` to the constant `8` — but inlining is a *fold* concern (post-infer),
> and infer has already rejected `mk`'s body by then. Pin a probe that `(def (mk n) (: 5 (UInt n)))` rejects
> CDZ0302 even when the only call is `(mk 8)`.

`build_int_ty` shared between fold-time and annotation-time ⇒ width validation cannot drift (the L2
anti-drift invariant, extended to naturals).

## 8. Stage 2 — bounds & conversions (`.max`/`.min`/`.of`/`.wrap`)

Two conversion intrinsics carrying the TARGET width (payload variants — `Intrinsic` need not stay `Copy`,
§0 non-issue):
```rust
Intrinsic::IntOf(IntTy),     // T.of  : α → T, CHECKED  — trap if the value is outside T's range
Intrinsic::IntWrap(IntTy),   // T.wrap: α → T, TRUNCATE — keep low N bits (T.wrap generalizes Int.to-byte)
```
`IntWrap{false,8}` *is* the existing `IntToByte` — systematize both under one rule (`(UInt 8).wrap` ≡
`Int.to-byte`); keep `IntToByte` as a resolve-time alias or migrate its one corpus use.

**Member access on a type-value** is the Stage-2 seam. `(. UInt8 max)`: `UInt8` → `TypeVal`; `(. (UInt 48)
max)`: operand is `Apply(Intrinsic(TypeUInt),[48])`, a *type-value only after fold*. Both must dispatch
`max/min/of/wrap`. Cleanest single mechanism: handle it in **infer's `RecordProj(field, e)` arm** — if `e`
types as `Ty::Type` and `extract_type_value(e)` yields an integer type `T`:
- `max`/`min` → a `TypedNode::Int(bits)` constant typed `Ty::Integer(T)` where `bits` = `T.max()`/`T.min()`
  as an i64 bit-pattern (covers `UInt8.max`=255, `Int8.min`=−128, `UInt64.max`=2⁶⁴−1 stored as −1 i64, and
  the width-indexed `(UInt 48).max` uniformly).
- `of`/`wrap` → the intrinsic value `IntOf(T)`/`IntWrap(T)`, typed `Fn([fresh α], Ty::Integer(T))` — a
  first-class conversion applied like any op.

This unifies aliases (`UInt8.of`) and width-indexed forms (`(UInt 48).wrap`) — both operands extract the
same `T`. Note this makes `RecordProj` on a type-value a *typing-level dispatch*, structurally like the
`Annot` extract; it declines (never mis-emits) for any non-integer type-value field. **This works only
because aliases are bare `TypeVal`s, not module records** — see the §6 load-bearing note (review finding
6): `member()` in resolve must fall through to `RecordProj` for `(. UInt8 max)`, which it does for a
non-record prelude entry (`resolve.rs:1010`) but would NOT if `UInt8` were an ops-record.

**Lowering (select).** `IntOf(T)`: range-check the (i64) operand against `T.min()/T.max()`, trap if
outside, else pass through. `IntWrap(T)`: mask to low N bits (unsigned) or sign-extend from bit N−1
(signed) — the generalized `Int.to-byte` sequence (select.rs:743 is the `{false,8}` instance).

## 9. Lowering — the width-parametric emitter (the operator's "right intrinsics")

**Representation, Stage 1: every width in an i64.** Uniform, simplest. An `(Int N)`/`(UInt N)` value is its
canonical magnitude in an i64: unsigned held non-negative (except `UInt64`, whose values above `i64::MAX`
are the natural i64 bit-pattern, read as u64); signed sign-extended from bit N−1. (`explicit-checked.md`'s
"i32 for N≤32" is a size optimization — deferred, §13.)

**The width rides on the type-annotated MIR slots that already exist**, not on bare `Mir::Int`. A scalar
`Mir::Int(255)` is width-agnostic; its width comes from its enclosing typed context: the function return
type (`entry_ret`, pipeline.rs:69 — carries the top-level width to render), `Mir::Let.value_ty`, a tuple
element's `(Ty,Mir)`, and the operand `IntTy` now on `Mir::Arith`/`Bit`/`Shift`/`Cmp`. This is consistent
with how the compiler already threads types (Int is I64 everywhere; the width matters only at
overflow-check, op-selection, and render, each of which reads an enclosing `Ty`/`IntTy`).

**`emit_checked_arith` generalized (select.rs:1005).** The existing signed-64 sequence *is* the
`IntTy::I64` instance of: (1) emit operands, raw `i64.add/sub/mul` into a scratch; (2) **range-check** the
result against `[T.min(),T.max()]` — for `I64` this is the existing overflow-bit test, for a narrower or
unsigned width it is a const-bounds compare `r<min ∨ r>max`; (3) out of range → `Unreachable`
(trap "integer overflow"); (4) leave the in-range result. Read `T` from the node's `IntTy`.

**Signedness selects the machine op** (all from the node's `IntTy`):
- `Cmp`: `signed ? {lt_s,gt_s,le_s,ge_s} : {lt_u,gt_u,le_u,ge_u}`. `UInt64` REQUIRES `lt_u`
  (`(< (:0 UInt64) UInt64.max)`=true — sharpest case: `UInt64.max` is −1 as i64, but the *largest* u64).
- `Shift` right: `signed ? shr_s : shr_u` (`(>> UInt8.max (:1 UInt8))`=127 needs `shr_u`). Left shift and
  div/rem range-checked like arith; the count guard (0..N else trap) is now per-width.
- `Bit` and/or/xor: raw i64 op then re-canonicalize into range (mask low N unsigned / sign-extend signed) —
  total, no range-check.

**⚠ Unsigned-64 is the sharpest correctness area.** Values in `2⁶³..2⁶⁴` are negative i64 bit-patterns;
fold and select arithmetic/compare on `{false,64}` must use **u64 / i128** interpretation, not signed i64.
`IntTy::wrap`/`fits`/`min`/`max` work in i128 for exactly this reason. Pin `UInt64.max` construction,
render, and `(< 0 UInt64.max)` explicitly.

**Bare/leaked handling** is unchanged from L2: an unapplied `TypeInt`/`TypeUInt`, or a `Mir::TypeVal` that
survives fold, declines at select's existing arms (select.rs:315,820); the erasure fence (CDZ0305) catches
a type-value smuggled into a compound.

**⚠ Boundary / observing a width-typed terminal value (review finding 2 — the biggest correctness gap).**
The corpus makes a *non-aliased-width value the program's observed output*: `:1186` (`(: 2⁴⁸−1 (UInt 48))`)
and `:1206` (`(UInt 48).wrap …` → a `(UInt 48)`). A non-aliased width has `comp_valtype = None`, and a
pure-scalar body takes the **scalar** component path (`layout.rs` `imports_runtime` is false for a plain
integer → the entry is lifted through `Ty::comp_valtype`, `serialize.rs:120–137`) → **the component can't
be built → uncoded decline → the harness scores TODO, never Pass** (`corpus.rs:431`). So as first drafted,
those two acceptance cases silently never pass. **Fix: route a width-typed terminal return through the
render-to-string (runtime-compound) path**, the same in-program rendering a compound result already uses —
force `imports_runtime` when the entry return type is an integer whose `comp_valtype` is `None` (a
non-aliased width). Then `render` (§ below) emits the `(: <v> (UInt 48))` string in-program, which crosses
the boundary as text. This is *also* the spec's answer to "what happens if you export a `(UInt 48)`"
(`explicit-checked.md:88–102` — a non-aliased width has no primitive ABI form; a program that must expose
one converts it to an aliased width, or — for an *observed terminal value*, which the corpus exercises —
renders it). Decide the exact trigger: minimally the non-aliased widths; arguably *any* observed width
(so the boundary form is uniform), since even an aliased-width scalar's canonical output form is
`(: v UInt8)`, not a bare `s8` (confirm against how the harness reads a scalar output — the value-compare
at `run_corpus.py` normalizes to the bare integer, so an aliased width MAY still take the scalar path and
compare its magnitude; the non-aliased widths are the ones that *must* render). Pin this with a probe on
`(: 2⁴⁸−1 (UInt 48))` producing the string, not a decline.

## 10. Fold — per-width constant overflow

Generalize `fold_arith`/`fold_bit`/`fold_shift` (fold.rs:821+) to read the operand `IntTy` off the node
and check the result against `[T.min(),T.max()]` in i128:
- outside → **CDZ0304 poison** (`poison("integer overflow in a constant operation")`), exactly as the
  Int64 case does today — Int64 is the `IntTy::I64` instance of this.
- inside → `Mir::Int(T.wrap(result) as i64)` (the canonical bit-pattern).

`Ty::Int`'s current `checked_add`/`checked_mul` behavior is the N=64 signed special case — verify
byte/behavior-identical. **⚠ corpus mismatch to fix in the governance act:** the width-overflow corpus
cases record only the dynamic `(trap …)` oracle, *not* `(compiler (error CDZ0304))`, because they were
written `(needs numeric-model)` before the capability was realized and before the const-trap-is-a-reject
ruling generalized to widths. A const-fold that *proves* the width overflow is the *correct* behavior;
when the capability is declared realized, those cases should gain `(compiler (error CDZ0304))` to match the
Int64 overflow cases (spec-fold follow-up, §12).

## 11. Diagnostics

`diag.rs` `enum Code` + `Code::code` (glosses per `options/diagnostics-schema/coded-span-record.md:72–73`):
```rust
NoPromotion,       // CDZ0301 — an operation on two different numeric types without an explicit conversion
WidthConstraint,   // CDZ0302 — a width outside 1..=64, or a non-constant (runtime) width
```
CDZ0303 (the `(pragma default-integer <T>)` non-integer check) belongs to the default-literal-type
sub-feature — **not** this task (Stage 3, needs module pragmas).

## 12. Verification & the litmus (READ THIS)

- **rcdzc unit tests + compile probes are the immediate signal** (the corpus cases stay skipped). Add:
  `(: 200 UInt8)`→200:UInt8; `(+ (:255 UInt8)(:1 UInt8))`→CDZ0304 (const) / trap (runtime); `(+ (:1 UInt8)
  (:2 Int32))`→CDZ0301; `(: 5 (UInt 65))`/`(UInt 0)`/`(UInt 128)`→CDZ0302; the runtime-width
  `(def (mk n) (: 5 (UInt n)))`→CDZ0302; `(: 5 (UInt 64))` ≡ `(: 5 UInt64)`; `(: (:5 (UInt 64)) Int64)`
  distinctness; `UInt8.max`=255, `Int8.min`=−128, `UInt64.max`=2⁶⁴−1; `(UInt8.of (:256 Int32))`→trap;
  `(UInt8.wrap (:-1 Int32))`=255; `(< (:0 UInt64) UInt64.max)`=true; `(>> UInt8.max (:1 UInt8))`=127.
- **THE regression pin:** an existing Int64 arithmetic case must emit **byte-identical** wasm after the
  `Mir::Arith`-carries-`IntTy` shape change and the `Ty::Int`→`Ty::Integer(I64)` rewrite. The `I64` path
  must not drift — this is where a single-representation refactor is riskiest.
- **Gate** (BEHAVIOR-GATE + IGNITION + cargo test; 0 FAIL + exit 0): the width corpus cases stay
  `(needs numeric-model)` = skipped, so the **FAIL set is unchanged** (verified: `corpus.rs:72–73` skips a
  case iff any `(needs X)` is unrealized, and `numeric-model` stays unrealized until the flip). Diff the
  FAIL set, not the P count (the standing drift trap). Landing this does not by itself flip those cases.
- **Governance act (separate, operator-gated, §2) — and it needs a RE-TAG (review finding 1).** The
  ~40 numeric-model cases share ONE `(needs numeric-model)` tag, but only 27 are width cases; the other 22
  (rational, BigInt, default-integer pragma, float, wrapping-type) are NOT implemented by this task.
  Realizing `numeric-model` wholesale un-skips all 49 → the 22 unimplemented ones RUN and FAIL (CDZ0101
  unbound-name on a runnable-primary case, `corpus.rs:420–430`) — ~15+ hard FAILs the design never claimed.
  So the flip is a **four-part** act:
  1. **Re-tag** the 27 width cases (`06-numeric-model.sexp:971–1265`) from `(needs numeric-model)` to a new
     finer capability `(needs integer-widths)`; leave the other 22 on `numeric-model`.
  2. Add **only** `integer-widths` to the realized set (`options/realized-capability-set/seed-ignition-set.md`),
     NOT `numeric-model` — so rational/BigInt/pragma/float stay skipped.
  3. Add the seven width→primitive rows to the **frozen** `component-abi.md` (additive — `Int64` keeps
     `s64` — but ABI-governed, version-incremented, not an incidental edit).
  4. Add `(compiler (error CDZ0304))` to the width-overflow cases (§10) so a proven-overflow reject matches
     the Int64 precedent.

  Recommend: land the rcdzc implementation behind the skip, verify by probes, THEN do this four-part act.

## 13. Staging summary

1. **Stage 1 (core).** `IntTy`, `Ty::Integer` (delete `Ty::Int`, rewrite 36 sites), `build_int_ty`,
   `TypeInt`/`TypeUInt`, resolve wiring + 16 aliases (as bare `TypeVal`s, §6), literal-adopts annotation
   (Model L), infer operand-typing + `require_integer` (CDZ0201 for non-numeric, CDZ0301 for width/sign
   mismatch) + literal-width shift count + annotation extract (literal width only), `unify` arm, per-width
   fold overflow, `IntTy` threaded onto arith/bit/shift, per-width select lowering, **width-typed terminal
   return routed through render-to-string (§9 finding 2)**, CDZ0301 + CDZ0302.
2. **Stage 2 (bounds & conversions, landing together per operator).** `.max`/`.min`/`.of`/`.wrap`,
   member-access-on-a-type-value dispatch, `IntOf`/`IntWrap` intrinsics + lowering.
3. **Later / separate acts.** Stage 3 `(pragma default-integer <T>)` (CDZ0303, needs module pragmas); the
   governance flip (§12); i32 packing for N≤32 (size optimization). Widths > 64 stay CDZ0302 (reserved to
   a future multi-word layer); `BigInt`/`Rational`/`Float64` arithmetic are distinct types, own tasks.

## 14. Open decisions for the operator

1. **Literal-typing model (§6)** — recommend **Model L** (bare literal is Int64; adopts the annotated
   width when it fits, at the annotation site). Model V (integer-kinded literal vars + finalize defaulting)
   is more textbook-HM but adds a kind system the seed HM lacks. This is the single most load-bearing
   decision in the design and the thing to scrutinize hardest.
2. **`extract_type_value` width-arg evaluation (§7, corrected by review finding 5)** — the width arg must
   be a **literal `Int`** in Stage 1+2, because infer runs before fold so there is no evaluator to call at
   annotation time. `(UInt (+ 4 4))` is deferred (→ CDZ0302 "non-constant width" for now). No corpus case
   needs a computed width; lifting it later means a small closed-integer evaluator callable from infer, or
   an annotation-extract-after-pre-fold. **Confirm you're content deferring computed widths.**
3. **`IntToByte` migration (§8)** — recommend keeping it as a resolve-time alias for `IntWrap{false,8}`
   rather than a duplicate lowering, and migrating its one corpus use in a follow-up.
4. **Boundary render trigger (§9 finding 2)** — force render-to-string for *only* non-aliased widths
   (minimal), or for *any* observed width (uniform `(: v UInt8)` output form)? Recommend confirming against
   how `run_corpus.py` value-compares a scalar output before choosing — it may allow aliased widths to keep
   the cheaper scalar path.
