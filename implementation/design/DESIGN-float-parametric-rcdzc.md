# DESIGN — Parametric floating-point for rcdzc

*2026-07-13. Operator ask: "get floating point implemented; parametric like ints; allow the sizes the
backend supports."* This doc pins the decisions and the increment plan. Read §0 first.

## §0 — Decisions (operator-confirmed 2026-07-13)

1. **Parametric type, mirroring `IntTy`.** A float type is `Ty::Float(FloatTy { width })`, where
   `FloatWidth ∈ { Fixed(u32), Deferred, Var(u32) }` and the default is 64 — the exact three-state
   shape of `Width`/`Sign` on integers. A float literal is `Ty::Float(FloatTy::deferred())`; it grounds
   to `Float64`. `Float32`/`Float64` are ALIASES for `(Float 32)`/`(Float 64)`, built by the same
   width-generic builder — nothing is special-cased per name (the `(Int N)` model).

2. **Realized widths = { 32, 64 }** — "the sizes the backend supports." A `(Float N)` with N ∉ {32,64}
   is rejected at compile time with **CDZ0302** (the same unsatisfied-width-constraint diagnostic
   `(UInt 0)` / `(UInt 65)` get). Both are realized END TO END now (f32 + f64 opcodes, boundary rep,
   conversions, rendering).

3. **Explicit FP operators, OCaml-style: `+.` `-.` `*.` `/.`** — top-level prelude operator records,
   one set (width resolved by the operand type / annotation, like the int operators are width-generic).
   The unqualified `+ - * /` stay INT-ONLY, so `(+ 2 2.0)` still rejects CDZ0301 (no silent promotion)
   and `(+. 2 2.0)` ALSO rejects CDZ0301 (`+.` is float-only — an int operand doesn't unify with
   `Float`). The bitwise/shift/`%` operators have no float form (a float is not a bit pattern). These
   tokenize as plain `Leaf::Name` in the s-expr surface (the number-parser needs a digit after the
   sigil; `is_dotted_name` needs non-empty segments) — no reader change. The ML/guide lexer needs a
   small addition, deferred (guide-only, no corpus impact).

4. **An out-of-range float literal is malformed → CDZ0201**, exactly parallel to the out-of-range
   integer literal `9223372036854775808`. This closes the 2026-07-08 spec gap (`1e400` → `inf`, which
   the reader can't read back): the language provides no `inf` spelling, so a literal that would round
   to a non-finite value is rejected at the reader boundary rather than silently saturating.

### Non-goals / deferred
- Float **ordering** (`<. >. …`) stays UNSPECIFIED (NaN has no total order under IEEE; the ordering
  learning 2026-07-05 already declines it). Float `=` is decided structurally on constants via the
  existing generic `= : ∀a. a → a → Bool` (bit-identical equality: `-0.0 ≠ 0.0`, all-NaN equal).
- `Rational` / `BigInt` are separate numeric tracks, untouched here.

## §1 — Current state (spec HEAD `191814c`)

`Ty::Float` exists as a MONOMORPHIC leaf (commit `005f99c`): a float literal has a type distinct from
`Ty::Int`, so `(+ 2 2.0)` rejects CDZ0301. But float VALUES do not run — `resolve` → `Resolved::Float`,
`infer` → `Ty::Float`, `lower::core_of` DECLINES ("a floating-point value does not yet run"). Runtime
`box-float`/`get-float` (f64) ops exist and are UNUSED; `CORE_F32/F64`, `COMP_F32/F64` valtype bytes
exist; `Leaf::Float(Decimal)` captures a literal EXACTLY (arbitrary-precision significand + base-10
exponent, no f64 rounding). f32/f64 ARITHMETIC opcodes are NOT in `wasm_abi.rs` yet (need the xtask
codegen op-list). The renderer must be byte-INJECTIVE (learning 2026-07-05: `f as i64` saturates).

## §2 — Increment plan (each: implement in worktree → 3 gates 0-FAIL → CAS land → 1 memory line)

- **F0 — spec-first.** In `numeric-model.md`: pin the concrete float widths {32,64} + default Float64 +
  the realized set; add `(Float N)` / `Float32` / `Float64` normative sentences (mirror the integer-
  width headings); state the explicit-FP-operator rule and the out-of-range-literal → malformed rule.
  In `06-numeric-model.sexp` + `01-literals.sexp`: rewrite the float cases to `+.`/`-.`/`*.`/`/.`; add
  `(Float N)` parametric-width + unsupported-width (CDZ0302) + out-of-range-literal (CDZ0201) cases.
  (These grade Todo until the value path lands — no gate regression: decline/unbound → Todo.)

- **F1 — parametric type layer (byte-neutral).** `Ty::Float(FloatTy{width})` replacing the leaf;
  `FloatWidth`. Wire unify/agrees_with/apply/occurs/freshen/has_free_var; render (`Float32`/`Float64`/
  `(Float N)`); `encode_ty`/`decode_ty` round-trip; `mismatch` — two DIFFERENT float widths → CDZ0301,
  Int-vs-Float → CDZ0301 (both already numeric). `ground_width` → 64 default. No values run yet →
  gate byte-neutral (the mixing cases stay Pass, pure-float stay Todo). The `IntTy` I3 keystone mirror.

- **F2 — prelude ctor + modules + FP operators.** `Prim::FloatCtor`; `Float` type-ctor record
  (`(meta apply)` = FloatCtor); `build_float_ty(N)` (N ∈ {32,64} else CDZ0302), the sole validator,
  shared by the annotation path + the ctor fold; `Float32`/`Float64` alias modules (each `(meta t)` =
  `(Float N)`, plus `max`/`min`/`of-int`/… fields as they land). Prelude records for `+. -. *. /.`
  (`OpShape::FloatBinary`, scheme `∀a. (Float a) → (Float a) → (Float a)`; `Prim::FAdd/FSub/FMul/FDiv`).
  Still no values run (fold/select decline) → the operators TYPE-check (so `(+. 2 2.0)` → CDZ0301) but a
  well-typed `(+. 1.0 2.0)` declines at lower until F3.

- **F3 — float const-fold + literal value crosses + renderer.** `Core::ConstFloat(f64 bits + width)`;
  `Decimal` → round-to-nearest-even at the target width (reject → CDZ0201 if it rounds to non-finite,
  per §0.4); `fold` for `FAdd/FSub/FMul/FDiv` + float `Eq` (bit-identical). Boundary: `valtype_of`/
  `comp_valtype_of` → f64/f32; the escape path renders the scalar. BYTE-INJECTIVE renderer in `cdz-run`
  (`{:.17e}`-class round-trippable form) + the gate's `float_output_round_trips` oracle. Flips `3.5`,
  `(+. 0.1 0.2)`, `1e19`, `-0.0`, `(= 1e19 1e20)`.

- **F4 — runtime float operands.** Add f32/f64 opcodes (`f64.add/sub/mul/div`, `f64.eq/ne`,
  `f64.promote_f32`/`f32.demote_f64`, `f64.const`/`f32.const`, `local` f64/f32) via the xtask codegen
  op-list (NOT by hand — the wasm_abi regen rule). `select` `Core::Arith`/`ConstFloat` → machine ops;
  runtime float params through an exported fn (`(def (add (: a Float64) (: b Float64)) (+. a b))`).
  Float ops NEVER trap (IEEE: overflow → inf, /0 → inf/nan) — so no overflow guard, unlike int arith.

- **F5 — conversions.** `Float64.of-int : Int64 → Float64` (+ per-width); `Float32.of`/`Float64.of`
  (f32↔f64 promote/demote); per the corpus. Int→float is EXPLICIT (no silent promotion).

- **F6 — Rust backend arms.** Every new `Core`/`Ty`/`Prim` variant needs a `backend/rust/expr.rs` arm
  (native `f64`/`f32`, `+`/`-`/`*`/`/`, literal, of-int as `as`). Exhaustive-match forces this; decline
  anything outside the realized slice. Run `xtask gate --target rust`.

## §3 — Key invariants / traps
- A new `Ty`/`Core`/`Prim` variant ⇒ arm in the RUST backend too (`backend/rust/expr.rs`), and a new
  ground type needs BOTH `encode_ty` AND `decode_ty` leaf arms (the Bytes round-trip bug).
- wasm opcodes are GENERATED into `wasm_abi.rs` via the xtask codegen op-list — add there, never by hand.
- Float const-fold rounds ONCE from the exact `Decimal` (no intermediate f64), so the canonical form is
  a function of the value, not of how it was written (deterministic-value-form contract).
- The renderer must be byte-injective over finite floats (the `f as i64` saturation defect); the gate's
  independent `parse::<f64>()` round-trip oracle keeps it honest.
