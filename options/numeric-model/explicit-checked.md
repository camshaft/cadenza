# Numeric Model — Choice: explicit-checked

> **The default choice for the `numeric-model` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the concrete numeric widths,
> representations, and modes that realize the numeric behavior the specification states
> technology-neutrally (numeric-model.md capability; deterministic-value-form.md §"Numeric
> Serialization"; determinism-and-fuel.md §"Floating-Point Emission Is Determinism-Constrained").
>
> Because this choice fixes bytes that cross the boundary and enter the canonical value form, a
> change to it is an ABI-level change under the constitution's Governance Floors.

## The default choices

| Concern | Default |
|---|---|
| Integer types | the two **width-indexed type constructors** `Int` and `UInt`, applied to a compile-time width `N` in `1..=64`: `(Int N)` is a checked two's-complement signed integer over `−2ⁿ⁻¹..=2ⁿ⁻¹−1`, `(UInt N)` a checked unsigned integer over `0..=2ⁿ−1`. Every application is a distinct, checked type; each traps on overflow of its own range, none silently converts to another width or signedness. |
| Default integer | **checked signed 64-bit** — `Int64`, the alias for `(Int 64)`; overflow **traps** deterministically rather than wrapping. It is the type a bare integer literal takes when nothing constrains it otherwise and the module declares no other default (see §"Default integer literal type"). |
| Arbitrary-precision integer | a distinct **`BigInt`** type of unbounded range, opted into explicitly — never overflows or wraps (see §"Arbitrary-precision integer"). |
| Module default literal type | a module MAY declare `(pragma default-integer <T>)` (a module directive, `options/module-pragmas/`) so a bare literal in it takes `<T>` instead of `Int64`; fixes a type, never a conversion; definition-site scoped (see §"Default integer literal type"). |
| Named width aliases | `Int8/16/32/64` and `UInt8/16/32/64` are ordinary aliases for `(Int 8)`…`(UInt 64)` — the common widths that also have a boundary representation; they are not separate primitives. Any other in-range width — `(UInt 48)`, `(UInt 62)`, `(Int 7)` — is an equally first-class type with no alias. |
| Integer conversions | **explicit** only: `T.of x` is a **checked** conversion (traps when `x` is outside `T`'s range), `T.wrap x` is a **truncating/wrapping** conversion (keeps the low `N` bits under `T`'s two's-complement representation). Neither is ever performed implicitly. |
| Wrapping integers | available as a **distinct type**, not a mode on the default integer, so wrap is never silent |
| Arbitrary-precision integer | a distinct **big-integer** type, opted into explicitly. A width `N` **above 64** is reserved to this multi-word layer, not to the width-indexed constructors above (see §"Widths above 64 are reserved"). |
| Floating-point | **IEEE-754 binary64**, single fixed rounding mode (round-to-nearest-even), no fused-multiply-add contraction, canonical not-a-number bit pattern |
| Exact rational | a **normalized pair of big-integers** (reduced to lowest terms, fixed sign convention), opted into explicitly |
| Numeric promotion | **none** — an operation on two different numeric types (including two different integer widths or signednesses) requires an explicit conversion |

## Integers are width-indexed, not a fixed set of primitives

The integer types are the two **width-indexed type constructors** `Int` and `UInt`, each a compile-time
function from a width to a type (type-system.md §"Generics Are Type-Valued Parameters" — "A generic type
constructor … MUST be a compile-time function from types to a type, applied by ordinary application").
`Int` and `UInt` differ only in that their parameter is a compile-time **natural width** rather than a
type: `(UInt 8)` is the unsigned 8-bit integer, `(Int 32)` the signed 32-bit integer, `(UInt 48)` an
unsigned 48-bit integer, `(UInt 62)` an unsigned 62-bit integer. There is no privileged set of eight
integer primitives the compiler special-cases; `Int64` is not a built-in distinct from `(UInt 48)`, it
is the *alias* `(Int 64)`. This is the same first-class-type machinery `(Option Int64)` and `(List T)`
already use, indexed by a width instead of by a type.

Every application `(Int N)` / `(UInt N)` for `N` in `1..=64` is a distinct, checked type: an operation
whose result leaves the type's range traps (`"integer overflow"`, numeric-model.md §"Overflow Is
Defined"), exactly as the default `Int64` does — no width wraps silently, and no signedness is silently
reinterpreted. `Int64` remains the type a bare literal takes when unconstrained; any other width is
reached by an annotation (`(: 200 (UInt 8))`, `(: 200 UInt8)`), a per-width bound (`(UInt 8).max` = 255,
`(Int 8).min` = −128, `(UInt 48).max` = 281474976710655), or a conversion.

**Named aliases for the common widths.** `Int8/16/32/64` abbreviate `(Int 8)`…`(Int 64)` and
`UInt8/16/32/64` abbreviate `(UInt 8)`…`(UInt 64)`. They are ordinary definitions
(`(def UInt8 (UInt 8))`), not primitives — they exist because those widths also have a boundary
representation (below) and are the ones a program writes most. An in-range width without an alias
(`(UInt 48)`, `(Int 7)`) is an equally first-class type; it simply has no shorter name.

**The width parameter is compile-time, resolved to a concrete natural.** `N` obeys the same fence every
type parameter does — type-system.md §"Generics Are Type-Valued Parameters": "A type parameter MUST be
resolvable to a concrete type at compile time, so that a type-value never flows from runtime data into a
position that determines a type." `(UInt some-runtime-value)` is rejected exactly as any other
runtime-valued type argument is: the width is always a compile-time constant. This keeps the feature at
**indexed types over compile-time naturals**, not dependent types — no runtime value ever determines a
type.

**The width constraint is an ordinary compile-time predicate.** `N` must be a natural in `1..=64`
(type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values"). `(UInt 0)`,
`(UInt 65)`, and `(UInt -3)` fail the constraint and are rejected at compile time with the
unsatisfied-constraint diagnostic (`CDZ0302`), the same way any generic instantiation whose argument
fails its parameter's constraint is rejected — not a runtime failure. The `1..=64` bound is exactly the
range that fits a single core-wasm integer register (below); widths above 64 are reserved to the
multi-word layer (§"Widths above 64 are reserved").

**Representation is computed from `N`, not selected from a table.** A value of `(Int N)` / `(UInt N)` is
stored in the smallest core-wasm integer that holds it — an `i32` for `N ≤ 32`, an `i64` for
`33 ≤ N ≤ 64` — and kept in its canonical range by masking to the low `N` bits (unsigned) or
sign-extending from bit `N−1` (signed) after each operation, with the overflow check comparing against
`±2ⁿ⁻¹` / `2ⁿ`. Computing the mask and bounds from `N` is *simpler* than a case analysis over a fixed
width set, and it is what makes an arbitrary in-range width free rather than another primitive to
hand-write. Signedness selects the machine operation — `div_s`/`div_u`, `lt_s`/`lt_u`, `shr_s`
(arithmetic, sign-filling) vs `shr_u` (logical, zero-filling) — and the overflow bounds. A `(UInt 64)`
can hold values above `Int64.max`; its comparisons and right shift are unsigned, so
`(< (: 0 (UInt 64)) (UInt 64).max)` is true where the same bit pattern read as `(Int 64)` would be
negative.

**Conversions are explicit and total-or-defined.** `T.of x` checks and traps when `x` does not fit `T`;
`T.wrap x` keeps the low `N` bits (`(UInt 8).wrap 256` = 0, `(Int 8).wrap 255` = −1,
`(UInt 48).wrap x` keeps the low 48 bits). The seed's `Int.to-byte` (truncate to `0..=255`) is exactly
`(UInt 8).wrap`, and `Float64.of-int` is the float analogue of `T.of`; the constructor systematizes both
under one naming rule rather than adding ad-hoc per-target primitives — and the rule is uniform across
every width, aliased or not.

**Boundary mapping (ABI-level) — only the aliased widths cross.** The component model provides integer
primitives at exactly eight widths — `s8/u8/s16/u16/s32/u32/s64/u64` — so only the aliased types have a
boundary representation: `(Int 8)→s8`, `(UInt 8)→u8`, `(Int 16)→s16`, `(UInt 16)→u16`, `(Int 32)→s32`,
`(UInt 32)→u32`, `(Int 64)→s64` (unchanged), `(UInt 64)→u64`. A non-aliased width like `(UInt 48)` has
**no boundary representation**, so — per component-abi.md §"Every Exported Type Has A Stable Boundary
Representation": "A type that has no defined boundary representation MUST NOT appear in an exported or
imported signature" — it is an **internal-only** type: excellent for bit-packing and field layout inside
a program, and converted to an aliased width (`((UInt 64).of x)`) to be exported. This needs no new ABI
surface beyond the eight standard widths. Because those eight fix bytes that cross the boundary and enter
the canonical value form, adding the seven new width→primitive rows (all but `s64`) is an **additive**
change under the constitution's Governance Floors and the deterministic-value-form contract's §"Additive
Evolution" (no already-serializable value changes form — `Int64` keeps `s64`), but it still touches the
**frozen** component-abi.md and MUST land through the coordinated governance act with a version
increment, not as an incidental edit. The rows to add to component-abi.md §"Every Exported Type Has A
Stable Boundary Representation" are those seven width→primitive pairs.

## Widths above 64 are reserved

A width `N` **greater than 64** is deliberately **not** a `(UInt N)` / `(Int N)` today: it fails the
`1..=64` constraint (`CDZ0302`) exactly as `(UInt 0)` does. The `1..=64` ceiling is the range a single
core-wasm register represents; a wider fixed-size integer (`(UInt 128)`, `(UInt 256)`) needs a
multi-word representation and multi-limb arithmetic, which is the **big-integer** layer's concern, not a
register-width type's. Reserving `N > 64` rather than rejecting the *notation* keeps the door open: a
later optional increment MAY lift the ceiling and realize wide fixed-size integers as a multi-word
representation, at which point `(UInt 128)` becomes a valid type with no change to the surface syntax —
only the constraint's upper bound and the representation move. Until then, a program needing more than 64
bits uses the opt-in arbitrary-precision big-integer type.

## Arbitrary-precision integer — `BigInt`, opted into explicitly

The arbitrary-precision integer type `BigInt` represents **every integer with no bound** — it is the
signed, unbounded companion of the fixed-width family, opted into explicitly like the wrapping and
rational types (numeric-model.md §"An Arbitrary-Precision Integer Has Unbounded Range", §"An
Arbitrary-Precision Integer Is A Distinct Type Opted Into Explicitly").

- **No overflow, ever.** A `BigInt` arithmetic operation **never traps for magnitude** and never
  wraps: its representation grows as the result requires, so `(* huge huge)` is just a larger `BigInt`.
  This is the whole point of the type — a domain that must not think about precision or overflow (a
  factorial, a cryptographic modulus, an exact accumulator) uses `BigInt` and the overflow question
  disappears. (Division by zero still traps: an unbounded range does not give `n/0` a value.)
- **Construction.** `(BigInt.of x)` is the explicit conversion from a fixed-width integer, and
  `Int64.of` / `(UInt N).of` convert *back*, **checked** — trapping when the `BigInt` is outside the
  target's range (`(UInt 8).of (BigInt.of 300)` traps, exactly as `(UInt 8).of 300` does). The
  canonical written value form is the ordinary decimal (`(: 42 BigInt)`); a `BigInt` and the `Int64`
  `42` are **distinct types** with distinct canonical forms, crossed only by an explicit conversion.
- **Distinct, no promotion.** No operation silently produces or consumes a `BigInt`: `(+ (BigInt.of 1)
  1)` mixes `BigInt` and `Int64` and is rejected (`CDZ0301`) exactly as an `Int64`/`Float64` mix is.
  The unbounded type does not "absorb" a fixed-width operand; the conversion is always written.
- **Representation and boundary.** A `BigInt` is a multi-word (multi-limb) signed magnitude. It has a
  boundary representation — a `list<u8>` in the fixed canonical two's-complement encoding pinned in
  `options/type-mapping/` — so unlike a non-aliased fixed width, a `BigInt` **may cross an exported
  signature**. That encoding fixes bytes that enter the canonical value form, so it is governed like
  every other boundary type.
- **Relationship to the reserved wide widths.** A width `N > 64` stays reserved (`CDZ0302`, §"Widths
  above 64 are reserved"); `BigInt` is the *unbounded* type, not a fixed 128- or 256-bit one. The two
  are different needs — `BigInt` grows without limit; a future `(UInt 128)` would be a fixed wide
  register type — and `BigInt` is the answer today for "more than 64 bits."

## Default integer literal type — a module may declare which integer a bare literal takes

By default a bare integer literal is `Int64` (the table above). A **module may declare a different
default** with the `default-integer` module directive (`options/module-pragmas/`), so that within that
module an integer literal with no other constraint takes `<T>` instead of `Int64`:

```
(module crypto
  (pragma default-integer BigInt)   ; bare literals in this module are BigInt
  (def (double x) (* x 2))          ; x : BigInt, 2 : BigInt, result BigInt — no overflow to think about
  (def (start) 1000000000000000000000))   ; the literal is a BigInt, no width worry
```

`pragma` is the module's compiler-directive channel (`options/module-pragmas/`): its key is drawn from
a pinned registry, and an unrecognized key is **rejected, not ignored** (`CDZ0601`) — a directive that
changes a program's meaning must never be silently dropped. `default-integer` is the registry's first
key. This is the ergonomic escape hatch your domains want: a module doing arbitrary-precision or
wrapping arithmetic throughout declares its default once, and every literal in it is born the right
type — no `(BigInt.of …)` around every constant. The design is deliberately narrow, so it buys
ergonomics **without** weakening any guarantee:

- **It fixes a type, not a conversion.** The declaration only changes what type an
  *otherwise-unconstrained* literal *starts as*. Once a literal has its type, **every no-silent-promotion
  rule applies unchanged** (numeric-model.md §"A Declared Default Fixes A Type, Not A Conversion"). In
  the `crypto` module above, `(+ (double x) someInt64)` is still `CDZ0301` — the default made the
  literals `BigInt`, it did **not** add any coercion between `BigInt` and `Int64`. This is why the
  feature is safe: it never introduces the silent promotion the model exists to forbid.
- **It applies at the definition site (lexical), never the use site.** The default in force for a
  literal is the one declared by the module the literal is *written in*, not by any module that imports
  it (numeric-model.md §"A Declared Default Applies At The Definition Site"). A function defined in an
  `Int64`-default module keeps its `Int64` literals when called from a `BigInt`-default module —
  **importing a module never changes the type of code inside it**, which is what keeps the feature
  compatible with separate compilation and deterministic meaning. The default is a property of the
  source region, resolved entirely before types leave the compiler.
- **An explicit constraint always wins.** An annotation or other constraint on a literal takes
  precedence over the module default (numeric-model.md §"A Declared Default Fixes A Type…", 2nd
  sentence): in a `BigInt`-default module, `(: 5 Int64)` is still `Int64`, and a literal in an
  argument position of a known `Int64` parameter is `Int64`. The default only decides the
  *otherwise-unconstrained* case.
- **Compile-time only, zero ABI impact.** The default is resolved during type-checking and then types
  erase (Principle VII); it changes which type a literal *has*, never a byte form or the boundary — a
  `BigInt` literal serializes as a `BigInt` and an `Int64` literal as an `Int64`, exactly as if each had
  been written explicitly. So the declaration is not an ABI concern; it is source-level ergonomics.
- **Any integer type is declarable**, not only `BigInt`: `(pragma default-integer Wrapping64)` for a
  module full of wrap-around hashing, `(pragma default-integer (UInt 32))` for a module of 32-bit index
  math. The directive names an integer type the numeric model admits; a non-integer type is rejected
  (`CDZ0303`, the numeric-domain check), distinct from an unrecognized pragma key (`CDZ0601`) or
  malformed pragma arguments (`CDZ0602`).

## Exact rational — a normalized pair of big-integers, opted into explicitly

The exact rational type `Rational` is a **normalized pair of big-integers** — a numerator and a
denominator — carrying a number with no loss of precision (numeric-model.md §"Exact Arithmetic Is
Exact", §"An Exact Rational Has A Canonical Normalized Form"). It is a distinct numeric type opted into
explicitly, exactly like the big-integer and wrapping types: no operation silently produces a
`Rational`, and none silently consumes one — the old Cadenza behavior where integer `/` yielded a
rational is precisely the silent promotion this model rejects.

- **Construction.** `(Rational.of n d)` builds the rational `n/d` from two integers, immediately
  normalized. `(Rational.of-int n)` is the whole rational `n/1`. The canonical *written* value form is
  `n/d` in lowest terms (`1/2`, `-3/4`, `5/1`), as the corpus records it.
- **Normalization is canonical.** A `Rational` is always kept in **lowest terms** (numerator and
  denominator share no common factor, reduced by their gcd) with a **fixed sign convention** — the sign
  lives on the numerator and the denominator is always strictly positive. So `(Rational.of 2 4)`,
  `(Rational.of 1 2)`, and `(Rational.of -1 -2)` are **one value** with one canonical byte form
  (`1/2`), and `(Rational.of 1 -2)` normalizes to `-1/2`. This makes rational equality structural over
  the normalized pair (deterministic-value-form.md §"A Value Has One Canonical Byte Form") and keeps the
  representation a function of the number, not of how it was written.
- **A whole rational is not a bare integer.** `(Rational.of-int 5)` is `5/1 : Rational`, a distinct
  type from `5 : Int64`; crossing between them is explicit (`Rational.of-int` in,
  `Rational.to-int`-style checked/truncating conversions out), never implicit — the same
  no-promotion discipline the integer widths obey.
- **Zero denominator has no value.** `(Rational.of 1 0)` denotes no number, so it **traps**
  (`"rational with zero denominator"`, numeric-model.md §"A Rational With A Zero Denominator Is Not A
  Value") rather than producing a value — the rational analogue of integer division by zero. This is a
  runtime trap on a runtime-computed denominator; a literal zero denominator a generation can see at
  compile time MAY additionally be rejected there.
- **Arithmetic is exact and closed.** `+`, `-`, `*`, `/` on two `Rational`s produce a normalized
  `Rational` with no rounding (`(+ 1/3 1/6)` = `1/2`), and rational `/` by a nonzero rational is total
  and exact (unlike integer `/`, which truncates, and float `/`, which rounds). Comparison
  (`<`, `>`, `=`, …) is exact over the normalized pair.
- **Representation.** Numerator and denominator are big-integers (the arbitrary-precision layer), so a
  rational never overflows; it is exact at any magnitude, paying the big-integer cost only when used.
- **Boundary.** `Rational` has **no primitive boundary representation** — like a non-aliased integer
  width, it is an internal-only exact type. A rational crossing an exported signature crosses as an
  explicit encoding a program chooses (e.g. a `{ num, den }` record of two big-integers, or a decimal
  string), never as an implicit ABI primitive.

## Relationship to units of measure

The dimensional layer (units-of-measure.md, `options/units-of-measure/`) is **generic over the
underlying numeric type `T`**: a quantity is `(Qty T u)`, and `Rational` is one admissible `T`. So the
two layers **compose orthogonally** — units track the *dimension*, rationals track the *exactness of
the magnitude* — and `(Qty Rational u)` is a quantity that is both dimensioned and exact. Dividing two
such quantities, the model's `/` divides the *magnitudes* by exact rational division (no float
rounding) **and** divides the *dimensions* by the unit-group quotient in one operation: `feet / seconds`
over `Rational` magnitudes yields an exact `(Qty Rational feet·second⁻¹)`. Because units erase before
emission and rationals do not, the erased value of `(Qty Rational u)` is exactly the underlying
`Rational` — the dimension is checked and discarded, the exact magnitude remains. This is the payoff of
keeping units generic over `T` rather than baking a fixed numeric type into the quantity: exactness and
dimensional safety are independent choices a program combines freely.

## Why these choices, against the north star

- **No silent promotion** (the sharpest departure from earlier Cadenza, where integer division
  yielded a rational): a surprising inferred type is a cost to both agent-writability and
  verification, and a hidden coercion path is a cost to reproducibility. Every conversion is
  explicit in the source, so an agent and a verifier both see exactly which arithmetic happens.
- **Checked-and-trapping default integer:** overflow is a defined, deterministic event, not
  undefined behavior and not a silent wrap; a program that needs wrap says so with a distinct type.
- **Width-indexed integers, not a fixed set of primitives:** the first Cadenza artifact is a *compiler*,
  and a compiler is the sharpest witness that one width does not fit — it manipulates `UInt8` (the bytes
  of a wasm module) and `UInt32` (section sizes, LEB128 operands, table and memory indices) constantly.
  Realizing only `Int64` forces that code to carry untyped 64-bit integers where it means a byte or a
  32-bit index and to mask by hand, which discards exactly the static typing the language promises. But
  the deeper point is that a *fixed* family of eight primitives repeats the very limitation this language
  set out to avoid: in a language whose types are first-class values and whose type constructors are
  ordinary compile-time functions, there is no reason a width should be a privileged built-in rather than
  a parameter. Making `Int`/`UInt` width-indexed means an unusual but genuinely useful width — a `(UInt
  48)` timestamp, a `(UInt 62)` tagged pointer, a `(UInt 24)` audio sample, a `(UInt 12)` ADC reading —
  is a first-class type the compiler *computes* (its mask, bounds, and ops all follow from `N`), not a
  wrapper-plus-hand-written-operators the author maintains. This is exactly the friction a fixed
  primitive set imposes elsewhere (a wrapped `u64` with by-hand masking and per-width operator impls);
  here the general form is *simpler* to implement than eight special cases, because the arithmetic is
  already width-parametric. The eight aliased widths are the subset the component-model boundary supplies
  as primitives, so no width crosses the boundary by an encoding the ABI does not already fix; all other
  widths stay internal, where the packing wins live.
- **Deterministic float mode:** fixed rounding, no FMA contraction, and a canonical NaN make
  floating-point results byte-identical across conforming runtimes, as the determinism contract
  requires.
- **Exactness where declared:** big-integer and rational are exact and opted into explicitly, so a
  program pays their cost only when it asks for exactness.

## Relationship to units of measure

Units of measure, if the optional dimensional-analysis capability is included, attach to numeric
values as a compile-time-only layer and are erased before emission (units-of-measure.md). They do
not change the numeric byte forms pinned here.
