# DESIGN — Dimensional analysis (units of measure) for rcdzc

*2026-07-13. Operator ask: "design dimensional analysis and get it implemented. Parametric over the
inner numeric type; a measurement family you join (`Length`) with registered units (`feet`, `meters`)
that auto-convert; automatic SI + IEC (bibi) prefixes; no runtime overhead — all compile-time static
analysis."* This doc pins the decisions and the increment plan. Read §0 first.

The **spec is already written** and rich — this is an IMPLEMENTATION plan against a fixed contract, not
a design-from-scratch:

- `spec/capabilities/units-of-measure.md` — the normative capability (checked-then-erased, mismatch is
  a compile-time error, families with exact scales, prefixes, optional layer).
- `options/units-of-measure/erased-compile-time-quantity.md` — the pinned choice: `(Qty T u)`, the
  free-abelian-group unit model, auto-conversion at a common unit.
- `spec/semantics/18-units-of-measure.sexp` — **29 corpus cases**, all currently grading `todo` (clean
  declines; the compiler does not crash on them, they pin the contract a realization must meet).

## §0 — Decisions

1. **`(Qty T u)` is one more compile-time-value-indexed type constructor** — structurally the same as
   `(Int N)` / `(Float N)` / `(List T)`. A quantity type is `Ty::Qty { inner: Box<Ty>, unit: Unit }`.
   The `unit` is a canonical exponent map (see §2). This rides the EXISTING `Apply(Prim)` type-builder
   path (like `ListCtor`/`MapCtor`); ZERO new IR-node variants beyond the `Ty` arm and the `Prim`s.
   (This is the L2 first-class-types model: a type constructor's `(meta apply)` builds a type.)

2. **Two-LAYER delivery, Layer 1 first** (operator-confirmed 2026-07-13):
   - **Layer 1 — the erasure-only dimensional CORE** (~15 of the 29 cases). Dimensions as a free
     abelian group over base dimensions; `+ - * / < = compare` become unit-aware; `Qty.of`/`Qty.value`
     construct/erase; the whole apparatus erases to `T` before emission (byte-identical). **Genuinely
     zero runtime cost** — Layer 1 never emits a scale multiply because it only handles one-unit-per-
     dimension (no conversion). New diagnostic **CDZ0501**. NO dependency on Symbols or Rationals.
   - **Layer 2 — FAMILIES, prefixes, auto-conversion** (~14 remaining cases). Needs TWO prerequisites
     that do not exist yet in the compiler: `Symbol` (`#"metre"`) and `Rational`. Deferred to their
     own verticals first (see §7). This is the `feet`/`meters` mixing + SI/IEC-prefix story.

3. **Dimensional equality is a canonical exponent-map compare — NO solver.** A unit is a `BTreeMap`
   from a base-dimension name to a signed integer exponent, with every zero-exponent entry dropped. Two
   units are the same dimension exactly when their maps are equal. `(Unit.* a b)` = map-add,
   `(Unit./ a b)` = map-subtract, `(Unit.^ u n)` = scalar-multiply-then-drop-zeros, `Unit.one` = the
   empty map. This is a finite, decidable, order-independent compile-time computation (the F#-units
   model), not a constraint search.

4. **Operator unit rules dispatch on the operator `Prim`, not on a name.** HM cannot express unit
   composition (`*` MULTIPLIES units — it does not unify them by equality), so after `infer` solves the
   operand types, a units-aware post-check reads the operator's `Prim` and applies its dimensional
   rule. This mirrors how `Prim::Wrap` already reads its target width off the application's solved type
   at lowering rather than baking it into the scheme — it stays inside the "no keys outside the
   prelude" discipline (dispatch is on the resolved `Prim`, never on `head == "+"`).

5. **Full erasure at `lower`.** `(Qty T u)` lowers to `T`; `Qty.of x u` lowers to (the lowering of)
   `x`; `Qty.value q` lowers to (the lowering of) `q`. A `Ty::Qty` NEVER reaches the backend —
   `lower`/`select` see only the inner `T`. The erasure fence (CDZ0305) already forbids a unit value
   crossing the boundary; a `Ty::Qty` at an export is stripped to its `T` (its comp-valtype is the
   inner type's), exactly as `component-abi.md` requires for a type with no boundary rep.

### Non-goals / deferred (Layer 1)
- **Families, named units, prefixes, auto-conversion** — Layer 2 (needs Symbols + Rationals).
- **`Qty.pow`** with a compile-time integer exponent — lands with Layer 1 if cheap, else Layer 2 (the
  `(Unit.^ u n)` map op is trivial; the surface `Qty.pow` can wait).
- The base-dimension NAME in Layer 1: since Symbols don't exist yet, Layer 1 names a base dimension by
  a **string** carried in the `(Unit.base #"metre")` position. ⚠ The corpus WRITES `#"metre"` (a symbol
  literal). See §6 for how Layer 1 handles this without a full Symbol type.

## §1 — Current state (spec HEAD)

- **`Ty`** (`ty.rs:193`) is a CLOSED, exhaustively-matched universe: `Int/Bool/Unit/Record/Tuple/List/
  Map/Bytes/String/Char/Float/Sum/Fn/Type/Var/Any`. Adding `Qty` touches every exhaustive match (§3).
- **`Prim`** (`resolved.rs`) already has the type-builder family (`IntCtor`/`UIntCtor`/`FloatCtor`/
  `ListCtor`/`MapCtor`/`SumCtor`/`TupleCtor`/`RecordCtor`/`FnCtor`) reduced by `eval::reduce_ctor` /
  `typeval_of`, and the value-erasing family (`Wrap` reads its target off the solved type).
- **Type-builder intrinsics** register in `prelude.rs` via `ctor_record(ast, PRIM)` (`Int`/`Float`/…);
  they reduce in `eval.rs` `apply_type`/`reduce_ctor` arms.
- **Operators** register as `operator_record(ast, op, OpShape)` carrying a `(meta t)` type-lambda +
  `(meta apply)` prim; typed generically by `infer::apply_type` (instantiate scheme, unify operands).
- **CDZ codes** (`diag.rs:21`): the highest band today is CDZ04xx (effects). CDZ0501 opens the CDZ05xx
  verification-layer band. `Code` enum + its `as_str()` arm both need the new variant.
- **NO units machinery exists** — `grep`'d clean (only rationale-comment mentions). All 29 corpus cases
  grade `todo`.
- **`Symbol` (`#"…"`) and `Rational` DO NOT exist** in the compiler — no lexer literal, no `Leaf`
  variant, no `Ty` variant, no prim. Both have design directions (`symbol-interning-direction`,
  `units-rationals-families-direction`) but zero code. This is why Layer 2 is gated behind them.

## §2 — The `Unit` representation

```rust
/// A UNIT is an element of the free abelian group over named base dimensions: a canonical map from a
/// base-dimension name to a signed integer exponent, with every zero-exponent base DROPPED. `Unit.one`
/// (dimensionless) is the empty map. Two units are the SAME DIMENSION iff their maps are EQUAL — a
/// finite, order-independent, solver-free compile-time comparison (units-of-measure.md §Dimensional
/// equality is decided by canonical exponent map). Backed by a `BTreeMap` so the canonical order is the
/// key order, making equality a plain `==` and cloning cheap (small maps). Held in `Ty::Qty`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Unit(std::collections::BTreeMap<String, i64>);   // Layer 2: key becomes resolved::Symbol

impl Unit {
    pub fn one() -> Unit { Unit(BTreeMap::new()) }
    pub fn base(name: impl Into<String>) -> Unit { /* single {name: 1} */ }
    pub fn is_dimensionless(&self) -> bool { self.0.is_empty() }
    pub fn mul(&self, other: &Unit) -> Unit { /* pointwise add, drop zeros */ }
    pub fn div(&self, other: &Unit) -> Unit { /* pointwise subtract, drop zeros */ }
    pub fn pow(&self, n: i64) -> Unit { /* scale each exponent by n, drop zeros (n=0 → one) */ }
    pub fn render(&self) -> String { /* metre·second⁻¹ style, for (: … (Qty …)) rendering */ }
}
```

The drop-zeros invariant is what makes `(Unit.* u (Unit.^ u -1))` == `Unit.one` structurally, and
`metre·metre` (`{metre:2}`) == `(Unit.^ metre 2)` (`{metre:2}`) the SAME dimension by `==` — the corpus
case "dimensional equality is decided by canonical exponent map, not written form".

## §3 — Layer 1 increments

### L1-0 — `Ty::Qty` through the closed universe (byte-neutral)

Add `Ty::Qty { inner: Box<Ty>, unit: Unit }` and fill EVERY exhaustive match. This is the standing
"a new `Ty` variant needs arms in ~13 places + a rust-backend arm" trap. Checklist (from `ty.rs`,
`unify.rs`, both backends):

- `ty.rs`: `has_free_var` (recurse into `inner`), `agrees_with` (inner agrees AND units equal),
  `join` (inner join, units equal else `Any`), `render_name` (`(Qty <inner> <unit.render()>)`).
- `unify.rs`: `unify` (Qty vs Qty → unify inners, REQUIRE unit equality else fail; Qty vs non-Qty →
  fail — a quantity never unifies with a bare number, the no-implicit-dimensionless-coercion rule),
  `apply` (recurse into inner), `occurs` (recurse), `freshen`/`rename` (recurse), `Subst::apply`.
- `eval.rs`: `encode_ty` + `decode_ty` — ⚠⚠ **BOTH arms are MANDATORY** (the B1-CONSTRUCTION bug: a
  missing `encode_ty` arm silently encodes as `Unit`, corrupting the round-tripped scheme → invalid
  wasm). A `Qty` encodes as `(Qty <inner-ty> <unit-node>)` where the unit node is the arena form of the
  exponent map; `decode_ty` reads it back. `typeval_of` builds a `Ty::Qty` from a `(Qty T u)` type
  expression.
- backends: `is_heap_type` (a Qty is heap iff its inner is), `valtype_of` / `comp_valtype_of` (a Qty's
  valtype IS its inner's — it's erased). Both the wasm backend (`select.rs`/`layout.rs`) and the RUST
  backend (`backend/rust/expr.rs`) need the arm — but see L1-3: after erasure at `lower`, a `Ty::Qty`
  should NEVER reach the backend, so the backend arm is a defensive `unreachable!`/inner-delegate.

Gate target: byte-identical (nothing constructs a `Qty` yet). Establishes the type through the universe
with zero behavior change — the "retrofit onto the closed universe, zero risk" increment.

### L1-1 — `Qty` / `Unit` prelude module + construction/observation

Register the prelude bindings (all via the existing `ctor_record`/`operator_record` mechanisms):

- `Qty` — a module record whose `(meta apply)` is `Prim::QtyCtor` (`(Qty T u)` in type position builds
  `Ty::Qty`) and whose fields are the operations:
  - `Qty.of : ∀(T,u). T → u → (Qty T u)` — `Prim::QtyOf`. Attaches unit `u` to `x`. Its result type
    reads the unit off the second argument (a compile-time unit value); see §4.
  - `Qty.value : ∀(T,u). (Qty T u) → T` — `Prim::QtyValue`. Recovers the inner numeric, discarding the
    unit (the explicit exit from the layer).
- `Unit` — a module record with:
  - `Unit.one` — `Prim::UnitOne`, the dimensionless unit VALUE (a compile-time unit).
  - `Unit.base : Symbol → Unit` — `Prim::UnitBase` (Layer 1: string-named, see §6).
  - `Unit.* : Unit → Unit → Unit` — `Prim::UnitMul`.
  - `Unit./ : Unit → Unit → Unit` — `Prim::UnitDiv`.
  - `Unit.^ : Unit → Int → Unit` — `Prim::UnitPow`.

A UNIT is a compile-time VALUE. It needs a compile-time representation the evaluator can build and
compare — `eval` reduces `(Unit.* a b)` etc. to a canonical `Unit`. Where does a `Unit` value live in
the value world? Two options (decide at L1-1):
  - **(a) A `Ty`-adjacent comptime value** carried like a type-value (`Resolved::TypeVal` already
    carries a `Ty`; add a sibling `Resolved::UnitVal(Unit)` OR fold a unit into `Ty::Type`'s world). A
    unit is erased-before-runtime exactly like a type, so it belongs in the same comptime tier.
  - **(b) Encode a unit AS a type** — a unit is only ever USED as the second index of `Qty`, so it can
    ride inside the `Ty::Qty.unit` field and never needs a standalone value. `Unit.one`/`Unit.base`
    produce a "unit type-value" — a `Ty::Type`-tagged node the evaluator reads as a `Unit`.
  **Recommendation: (b)** — a unit is a type-index, not a first-class runtime value; keeping it inside
  the type world avoids a new value tier. `Prim::UnitOne`/`UnitBase`/… reduce (in `eval`) to a
  comptime unit the `QtyCtor`/`QtyOf` arms read, the same way `IntCtor` reduces a width to a `Ty`.

### L1-2 — operator unit rules (`+ - * / < = compare`)

After `infer` solves operand types, a units post-check applies the dimensional rule per operator
`Prim`. Where: extend the operator-application typing in `infer::apply_type` / the `type_errors` pass so
that when an operand is a `Ty::Qty`:

- `Add`/`Sub`: BOTH operands must be `Qty`, inner `T` must unify (numeric core unchanged — an `Int64`
  quantity + a `Float64` quantity is still CDZ0301), and units must be EQUAL (Layer 1: no conversion) —
  else **CDZ0501**. Result = `(Qty T u)`.
- `Lt`/`Gt`/`Le`/`Ge`/`Eq`/`Compare`: units must be equal (dimensions comparable) — else CDZ0501.
  Result = `Bool` / `Ordering` (unchanged).
- `Mul`: result unit = `a.unit.mul(&b.unit)`; inner `T` unifies. Result = `(Qty T (u_a · u_b))`.
  Multiplying by a dimensionless `(Qty T Unit.one)` keeps the dimension (empty-map add).
- `Div`: result unit = `a.unit.div(&b.unit)`. `(/ (Qty 6 metre) (Qty 2 metre))` → `Unit.one`
  (dimensionless) by map cancellation.
- A mix of `Qty` and bare numeric (`(+ (Qty 1 metre) 1)`): CDZ0501 (or CDZ0203) — no implicit
  dimensionless coercion, matching the no-silent-promotion stance.

⚠ Because `Mul`/`Div` PRODUCE a new unit rather than constrain one, this cannot be a pure `unify` — it
is a post-solve computation that reads both solved operand units and CONSTRUCTS the result unit. This is
the one place units step outside HM; it dispatches on `Prim`, reads solved `Ty::Qty`s, builds a
`Ty::Qty` result. Keep it in a dedicated `units.rs` helper called from `apply_type` (value column) and
`type_errors` (the CDZ0501 rejection), so the rule lives in one place.

### L1-3 — erasure at `lower` + the annotation path

- `lower`: `Prim::QtyOf` lowers to the lowering of its VALUE argument (the unit arg is dropped);
  `Prim::QtyValue` lowers to the lowering of its quantity argument. The arithmetic on quantities lowers
  to the PLAIN `T` operation (the `Core::Arith` the inner numeric already produces) — the unit adds
  nothing to the emitted code. A `Ty::Qty` is never materialized at runtime.
- `(: e (Qty T u))` annotation: the ordinary `Hir::Annot` path (transparent, mismatch → CDZ0203) plus
  the dimensional specialization — an annotation at a dimension the expression does not DERIVE is
  **CDZ0501** (the corpus "annotating a quantity at a dimension the expression does not derive"). The
  annotation reduces `(Qty T u)` to a `Ty::Qty` via `typeval_of` and unifies; a unit mismatch is
  CDZ0501, a `T` mismatch stays CDZ0203.
- The corpus records terminal outputs as `(: (Qty.of 5.0 metre) (Qty Float64 metre))` — so the RESULT
  RENDERER must render a `Ty::Qty` (`render_name`) and a quantity terminal value renders as its
  construction form. Check `08-value-rendering` conventions; a quantity's VALUE is its inner value with
  the unit type in the `(: … T)` position (the value is byte-identical to the bare inner).

Gate target after L1-3: the ~15 Layer-1 corpus cases flip `todo`→`pass`. These are the cases using ONLY
`(Unit.base #"…")` (no `Unit.of`/`Unit.prefix`, no `Rational`):

- a quantity is constructed from a numeric value and a unit
- Qty.value recovers the underlying numeric value
- a dimensionless quantity carries the group identity Unit.one
- adding two quantities of the same dimension keeps that dimension
- adding / subtracting quantities of incompatible dimension is a compile-time error (×2)
- multiplying quantities multiplies their dimensions
- dividing quantities divides their dimensions
- scaling a quantity by a dimensionless quantity keeps its dimension
- a unit multiplied by its own inverse cancels to the dimensionless unit
- comparing two quantities of the same dimension yields a Bool
- comparing / equality across incompatible dimension is an error (×2)
- dimensional equality is decided by canonical exponent map, not written form
- annotating a quantity at a dimension the expression does not derive is an error
- a quantity's erased value is the identical numeric value the bare literal has
- the underlying numeric type obeys the numeric core — no silent promotion under a unit (CDZ0301)
- a function deriving a velocity from a distance and a time (runtime-carried quantity through a fn)

That's ~18 cases — the whole `(Unit.base …)` core, INCLUDING the runtime-carried-through-a-function
payoff case (the dimension is checked at the def and erased; the compiled `speed` is plain division).

## §4 — How `Qty.of` reads its unit (the one subtlety)

`Qty.of : ∀(T,u). T → u → (Qty T u)` — the RESULT type's unit is the VALUE of the second argument, a
compile-time unit. This is like `SumNew` reading its discriminant off the ctor's `(meta variant)` at
lowering, or `Wrap` reading its target width off the solved type: the unit is not a type VARIABLE
unified in the ordinary way; it is a comptime value the `QtyOf` arm READS to construct the result
`Ty::Qty`. So `apply_type`/`type_errors` get a dedicated `Prim::QtyOf` arm: solve arg0's `Ty` = the
inner `T`, evaluate arg1 to a `Unit` (via `eval`), build `Ty::Qty { inner: T, unit }`.

The corpus writes the unit inline (`(Qty.of 5.0 (Unit.base #"metre"))`) so the unit is always a
compile-time-constructible expression — `eval` reduces `(Unit.base #"metre")` / `(Unit.* a b)` /
`(Unit.^ u n)` to a canonical `Unit` with no runtime residue.

## §5 — CDZ0501

New `Code::DimensionMismatch => "CDZ0501"` in `diag.rs` (enum variant + `as_str` arm). It opens the
CDZ05xx verification-layer band. Emitted by the units post-check for:
- `+`/`-`/comparison/`=` across unequal dimensions;
- an annotation at a dimension the expression does not derive (the dimensional specialization of
  CDZ0203 — CDZ0203 names the general type conflict, CDZ0501 names it when the conflict is dimensional).
There is NO runtime trap (units erase before runtime), so CDZ0501 is always a compile-time rejection —
it sits in the verification band, not the numeric-trap band.

## §6 — The `#"metre"` symbol-literal problem in Layer 1

The corpus writes base dimensions as SYMBOL LITERALS: `(Unit.base #"metre")`. The `#"…"` literal does
NOT exist in the lexer/reader yet (no `Leaf::Sym`, no lexer rule). Layer 1 has two honest options:

- **(a) Add a minimal `#"…"` reader literal that lexes to a `Leaf::Str` (or a new `Leaf::Sym(String)`)**
  — enough for `Unit.base` to read the name as a string. This is a SLICE of the Symbol vertical (just
  the literal + a string-backed leaf, no intern table, no `Ty::Symbol`, no `Symbol.of`/`to-string`).
  Layer 2 later promotes the leaf to a real `Ty::Symbol`. **Recommendation:** do this — it's small
  (lexer + one `Leaf` arm + printer), unblocks the corpus AS WRITTEN, and is a genuine down-payment on
  the Symbol vertical. The base-dimension name is then a `String` inside `Unit` for Layer 1; the map
  key becomes `resolved::Symbol` when Symbols land.
- (b) Rewrite the Layer-1 corpus cases to name base dimensions with bare names. REJECTED — it would
  diverge the corpus from the spec's written surface and create churn when Symbols land.

⚠ SPEC-FIRST NOTE: the `#"…"` literal reader is a syntax addition. It has an options record
(`options/symbol-interning/`) and corpus (`17-symbols.sexp`, all `needs symbols`). Adding just the
reader literal (not the full Symbol type) is additive and does not realize the Symbol capability — the
`17-symbols` cases stay `todo`. Keep the reader-literal change minimal and separate from the units
commits so it's clear it's a shared primitive.

## §7 — Layer 2 (deferred — needs Symbols + Rationals)

Not in scope now; recorded so the increment boundary is clear. Layer 2 delivers the remaining ~11
family/prefix/mixing cases and requires two prerequisite verticals FIRST:

1. **Symbols** — a real `Ty::Symbol` + `Symbol.of`/`Symbol.to-string` + intern table (design:
   `symbol-interning-direction`, corpus `17-symbols.sexp`). The `Unit` map key becomes `Symbol`.
2. **Rationals** — a real `Ty::Rational` (or a rational numeric value) + normalization + zero-denom
   trap (design in `units-rationals-families-direction` + `options/numeric-model/explicit-checked.md`,
   corpus cluster in `06-numeric-model.sexp`). Exact scales (`inch` = 127/5000, `milli` = 1/1000) and
   exact mixing (`1 inch + 1 mm` → 33/1250 m) are exact ONLY over Rational.

Then Layer 2 adds:
- A FAMILY = a dimension + a reference unit + sibling units each with an exact `Rational` scale to the
  reference. The prelude supplies SI families + common imperial/information units as ORDINARY data (a
  dimension symbol + a `unit-name ↦ Rational-scale` map); a program MAY declare its own.
- `Unit.of #"inch"` (a named family unit), `Unit.prefix kilo metre` (a scaled unit), `Unit.in u q`
  (explicit conversion).
- SI decimal prefixes (`kilo` 10³ … `pico` 10⁻¹²) + IEC binary prefixes (`kibi` 2¹⁰ … `tebi` 2⁴⁰), each
  an exact `Rational` scale value. `kB` (1000) and `KiB` (1024) are DISTINCT units, never equated.
- **AUTOMATIC exact conversion** when two operands share a dimension but differ in unit: each converts
  to the dimension's REFERENCE unit by its exact scale, combines there. The result unit is the
  reference (deterministic, evaluation-order-independent). This is the ONE place a scale multiply is
  emitted — const-folded when the magnitudes are constant (zero runtime cost), emitted only when a
  magnitude is a runtime value. The HONESTY AMENDMENT: "zero runtime cost" is total for Layer 1 and for
  same-unit Layer 2; a runtime-valued MIXED-unit combine emits the exact-scale `Rational` multiply the
  source denotes by naming two units (units-of-measure.md §A Unit Conversion Is The Arithmetic The
  Source Denotes).

## §8 — Test / gate strategy

- Each increment: `cargo xtask gate` (behavior), `cargo test -p rcdzc`, `cargo xtask check`. Bar = 0
  FAIL + exit 0 across the three gates. ⚠ Build the runtime FIRST (stale-runtime false alarms); no
  `CARGO_TARGET_DIR` on gate runs.
- L1-0 is byte-neutral (nothing constructs a Qty) — verify the pass count is UNCHANGED.
- L1-3 flips the ~15–18 Layer-1 corpus cases `todo`→`pass`. ⚠ Verify via `gate --case "<substr>"`
  reading `actual:`, not by name.
- ⚠ A `todo`→`FAIL` flip anywhere is a MISCOMPILE (a wrong answer) — investigate before landing.
- Land in a `.claude/worktrees/` worktree, merge to `spec` via the guarded-CAS block; NEVER edit the
  main tree (spec is checked out there). Repair main's phantom-stale working copy after landing.

## §9 — Risk register

- **The `Ty::Qty` arm sprawl** (~13 match sites + 2 backends). Mitigated by L1-0 being byte-neutral and
  exhaustively-checked by the compiler — a missing arm is a compile error, not a silent bug. The ONE
  silent trap is `encode_ty`/`decode_ty` (a missing arm mis-encodes as `Unit`) — write BOTH first and
  add a round-trip unit test for `Ty::Qty` before anything else reads it.
- **The `Mul`/`Div` unit-composition rule steps outside HM.** Isolated in `units.rs`, dispatched on
  `Prim`, reading solved `Ty::Qty`s — never a name match, so it stays inside the prelude discipline.
- **`#"…"` reader literal** touches the front-end (a syntax addition). Keep it minimal (leaf + lexer +
  printer round-trip) and in its own commit; it's a shared primitive with the Symbol vertical.
- **Layer 2 is genuinely blocked** on Symbols + Rationals — do NOT half-build families with strings and
  floats (float scales round; the spec REQUIRES exact rationals). Land Layer 1 cleanly, then the two
  prerequisite verticals, then Layer 2.
