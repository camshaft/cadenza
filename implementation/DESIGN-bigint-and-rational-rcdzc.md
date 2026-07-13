# DESIGN — Arbitrary-precision `BigInt` (and `Rational`) for rcdzc

*2026-07-13. Operator directive: "we need to build a bigint arbitrary precision type in the runtime as a
prerequisite. do deep research on the best way to do that and write up a document for design and impl
plan."* This is the design + increment plan.

> **STATUS (2026-07-13):** B0 (`Ty::BigInt` through the closed universe) and B1 (`BigInt.of` +
> constant folding + checked `Int64.of`-back) are LANDED on `spec`. The RUNTIME limb library
> (`cdz-runtime/src/bigint.rs`: add/sub/mul/divmod/gcd/cmp + boundary encodings) is being built in
> PARALLEL by separate cron-driven agents (landed `05dbcc46`/`cbda83a0`/`4e577d7c`).
>
> ⚠ **B2 (`Ty::Rational`) is PAUSED pending a cross-track sync (operator-directed).** Rationals need
> bignum, and there are TWO bignum layers with DIFFERENT owners: the RUNTIME `Big` (the crons' crate,
> for runtime-valued rationals) and a COMPILER-SIDE bignum for constant folding. `rcdzc`'s own
> `IntValue` (`ast.rs`) has NO mul/div/gcd — so B2's normalization needs either (a) reach-through to
> `num_bigint::BigInt` (already a transitive dep via `cadenza-syntax`) or (b) new arithmetic on
> `IntValue`. Building the compile-time-fold layer now risks DUPLICATING / conflicting with whatever the
> BigInt track intends to provide compiler-side. **Open question for the sync: does the BigInt track
> plan a shared compiler-side rational/bignum-arithmetic surface, or should the units track build B2's
> `num-bigint`-backed constant fold independently?** Resolve before resuming B2. (There is NO live
> channel between the tracks — coordination is via `spec` commits + this doc.)

## §0 — Why, and what this unblocks

The units-of-measure corpus has **9 remaining `todo` cases** (`spec/semantics/18-units-of-measure.sexp`),
and every one of them declines for exactly one reason: its magnitude is an **exact `Rational`**
(`(Rational.of n d)`), and `Rational` does not exist in the compiler. `Rational`, per the spec, is *a
normalized pair of big-integers* (`options/numeric-model/explicit-checked.md` §"Exact rational"), so
`Rational` cannot be built without `BigInt` first. Hence: **`BigInt` is the prerequisite; `Rational` sits
on top; the 9 units cases fall out of `Rational`.**

⚠ The operator explicitly considered and REJECTED the shortcut of a machine-int (i128) `Rational`
stopgap ("we definitely don't have arbitrary precision support"; "you'd need Rational to be parametric…
once we get big int"). Every magnitude in those 9 cases *does* fit i128, but an i128-only `Rational`
would silently overflow on a larger real program — the dishonesty the operator's rule "do NOT half-build
families" forbids. So we build the real unbounded type. This document is that build's plan.

## §1 — The authoritative surface (already in the spec)

The surface is **already fully specified** — this is an implementation plan against a fixed contract,
not a design-from-scratch. The normative sources:

- **`spec/capabilities/numeric-model.md` §"Arbitrary Precision"** (the MUSTs):
  - *An arbitrary-precision integer MUST represent every integer with no bound* — an operation on it
    *MUST NOT trap for the magnitude of its result*, growing its representation as needed (never wraps,
    never overflow-traps).
  - *An arbitrary-precision integer MUST be a distinct numeric type* — it does not silently convert
    to/from a fixed-width integer without an explicit conversion.
- **`spec/capabilities/numeric-model.md` §"Exactness"** (Rational MUSTs): canonical normalized form
  (lowest terms + fixed sign), and a zero denominator *is not a value* (fails at a defined point).
- **`options/numeric-model/explicit-checked.md` §"Arbitrary-precision integer — `BigInt`"** and
  §"Exact rational" (the concrete surface — reproduced in §2/§7 below).
- **`options/type-mapping/component-model-types.md`** (the pinned boundary encoding):
  - Big-integer → `list<u8>` in a **fixed canonical two's-complement encoding**.
  - Rational → `record { numerator: list<u8>, denominator: list<u8> }`, normalized.

### The `BigInt` surface (from explicit-checked.md §"Arbitrary-precision integer")
- **No overflow, ever.** `+ - * /` never trap for magnitude, never wrap. (Division by zero still traps —
  unbounded range does not give `n/0` a value.)
- **Construction/conversion.** `(BigInt.of x)` converts *from* a fixed-width integer. `Int64.of` /
  `(UInt N).of` convert *back*, **checked** — trapping when the `BigInt` is outside the target range
  (`(UInt 8).of (BigInt.of 300)` traps, exactly as `(UInt 8).of 300` does). Canonical written form is
  ordinary decimal (`(: 42 BigInt)`).
- **Distinct, no promotion.** `(+ (BigInt.of 1) 1)` mixes `BigInt` and `Int64` → **rejected `CDZ0301`**,
  exactly as an `Int64`/`Float64` mix is. No operation silently produces or consumes a `BigInt`.
- **Boundary rep.** A `BigInt` has a boundary representation (`list<u8>` two's-complement) — so, unlike a
  non-aliased fixed width, it **may cross an exported signature**.
- **Relationship to reserved widths.** `N > 64` stays reserved (`CDZ0302`); `BigInt` is the *unbounded*
  type, not a fixed 128/256-bit register type.
- **Default-literal pragma (LATER, out of this plan's core scope).** `(pragma default-integer BigInt)`
  makes bare literals `BigInt` in a module. Fixes a type, not a conversion; lexical; explicit constraint
  wins. Depends on the module-pragma channel (`CDZ0601`/`CDZ0602`/`CDZ0303`) — a SEPARATE increment,
  noted in §8, not blocking the core BigInt/Rational vertical or the units cases.

## §2 — Where the pieces already exist (leverage, don't rebuild)

Two large pieces already exist and must be reused:

1. **The compiler ALREADY has arbitrary-precision integers.** `cadenza-syntax` depends on `num-bigint`
   0.4 (`Cargo.toml:22` — "arbitrary-precision Int and the Decimal significand"), and `ast::IntValue`
   is a bignum. So **all COMPILE-TIME constant folding of `BigInt` literals and operations reuses
   `num-bigint`** — `(+ (BigInt.of 2) (BigInt.of 3))` on constants folds in the compiler with zero
   runtime cost, exactly as fixed-width constant arithmetic already does. The runtime bignum is needed
   ONLY for values not known at compile time (a `BigInt` parameter, an accumulator in a loop).

2. **The unit-scale machinery is already an exact ratio.** `ty::Unit` carries `(scale_num, scale_den)`
   as `i128` with `normalize_ratio`/`gcd_i128` (`ty.rs`). `Rational` normalization is the SAME algorithm
   over `BigInt` instead of `i128` — the gcd-reduce + sign-fix logic ports directly.

### The runtime heap model the BigInt leaf plugs into (from the runtime map)
`cdz-runtime/src/lib.rs` — a **tagless** reference-counted heap. A `Node` is `{ rc: u32, handles:
Handles, raw: Raw }` with **no type tag** (the compiler holds the static type). Key facts:
- **A BigInt is just a raw-bytes leaf with ZERO child handles** — structurally identical to a `Bytes`
  leaf (`op_bytes_alloc`/`op_bytes_get`, `lib.rs:1301-1373`). No new `Node` field, no discriminant —
  follow the Bytes precedent.
- `Raw` is inline-or-spill (`INLINE_RAW_CAP = 12`); a real bignum (>12 payload bytes) lives in
  `Raw::Heap(Vec<u8>)`, which is fine.
- Refcount: `op_dup` = `rc += 1`; `op_drop` on a zero-handle leaf just frees the box (the cheapest node
  shape — no child cascade). A BigInt leaf needs no special dup/drop.
- Immediates: a small integer that fits a **30-bit signed window** (`FIXNUM_MIN/MAX`, `lib.rs:604`) is a
  tagged immediate, not a heap node. `op_box_int` (`lib.rs:806`) normalizes on construct: in-window →
  immediate; else an 8-byte LE `Raw` leaf.

## §3 — Decisions

1. **`Ty::BigInt` is a new, nullary ground type** — like `Ty::String`/`Ty::Bytes`, NOT a width-indexed
   `Ty::Int`. It is a CLOSED-universe addition: every exhaustive `match` on `Ty` gets a `BigInt` arm
   (`has_free_var`/`agrees_with`/`join`/`render_name`/`ground_type`/unify/backend). Distinct from every
   fixed width — `agrees_with` is TRUE only `BigInt`↔`BigInt`, so an `Int64`/`BigInt` mix is a mismatch
   (`CDZ0301`), satisfying "distinct, no promotion" for free from ordinary HM.

2. **The runtime representation is a sign-magnitude limb-array leaf** (raw bytes, zero handles). Layout
   (little-endian, packed in `Raw`):
   - byte 0: sign (`0` = non-negative, `1` = negative); zero is always sign `0` (canonical).
   - bytes 1..: the magnitude as little-endian bytes with **no trailing zero bytes** (canonical — a
     magnitude of 0 is the empty tail, so the value `0` is exactly `[0x00]`).
   This is a canonical byte form (deterministic, one representation per value) — REQUIRED because
   `champ_hash`/`champ_eq`/`value-eq` compare raw bytes, so a `BigInt` used as a map key or compared
   with `=` must have exactly one byte form. (Rationale for sign-magnitude over two's-complement
   INTERNALLY: add/sub/mul/div/gcd are simpler and the canonical-normalization — strip trailing zeros —
   is trivial. The BOUNDARY encoding is two's-complement `list<u8>` per type-mapping; convert at the
   boundary op only, §6.)

3. **Compile-time constant BigInts fold in the compiler via `num-bigint`; only runtime-valued BigInts
   touch the runtime limb code.** `Core::ConstBigInt(num_bigint::BigInt)` is the folded form (mirrors
   `Core::ConstInt`). A constant `BigInt` that crosses to the runtime is emitted as its canonical
   sign-magnitude bytes via a `bigint-const`/`bigint-of-bytes` runtime op (or reuses `bytes-alloc` +
   `bigint-of-bytes`). So the runtime arithmetic ops are exercised ONLY by genuinely-runtime operands —
   and, critically, the 9 units cases are ALL compile-time constants, so **they fold in the compiler and
   need NONE of the runtime limb arithmetic** (see §9 — this lets the units cases land on a thin runtime
   slice).

4. **Runtime bignum arithmetic is a small hand-written `no_std` limb library IN `cdz-runtime`, NOT a
   dependency.** ⚠ The runtime's wasm bytes are content-hashed (`REQUIRED_RUNTIME_HASH`) and it is
   `#![no_std] + alloc` with its own talc allocator. `num-bigint` *does* support `no_std`
   (`default-features=false`), but pulling it (+ `num-integer` + `num-traits`) into the frozen runtime is
   a large, hard-to-audit hash-changing dependency. The runtime bignum surface is small (add, sub, mul,
   divmod, cmp, from/to-bytes, from/to-i64) over `Vec<u32>`/`Vec<u8>` limbs — hand-write it in a new
   `cdz-runtime/src/bigint.rs` module (schoolbook algorithms; the magnitudes real programs hit are small,
   and correctness > asymptotics for the seed). This keeps the runtime self-contained and auditable, and
   the module is independently unit-testable natively.

5. **The boundary crossing is two's-complement `list<u8>`** (pinned, type-mapping). A `BigInt` export
   lowers to the value-encode/decode path already used for compound escapes: a dedicated
   `bigint-to-bytes`/`bigint-of-bytes` pair converts the internal sign-magnitude leaf ↔ the canonical
   two's-complement `list<u8>`. (The internal form is sign-magnitude for arithmetic; the boundary form is
   two's-complement for the ABI — the conversion is O(limbs) and lives in the two boundary ops.)

## §4 — The `Ty::BigInt` type + prelude surface

- **`Ty::BigInt`** — new nullary ground `Ty` variant. Arms in every exhaustive match (the compiler forces
  this — a missing arm is a compile error, not a silent bug; the ONE silent trap is `encode_ty`/
  `decode_ty` for the typed-scheme round-trip, so write BOTH first + a round-trip unit test, per the
  `Ty::Qty` lesson).
- **`BigInt` prelude module** — a record like `String`/`Char`: `(meta t) = (intrinsic "BigInt")` so bare
  `BigInt` in type position IS the type; plus fields:
  - `of` — `Prim::BigIntOf` : `∀(N). (Int N) → BigInt` (checked-free widening from any fixed width;
    exact, never traps — every fixed-width int fits).
  - (the reverse `Int64.of`/`(UInt N).of` from a `BigInt` — a CHECKED narrowing that traps out-of-range —
    is added to the EXISTING integer modules' `of`, dispatching on a `BigInt` argument. `Prim::CheckedOf`
    already exists for fixed-width checked conversion; extend it to accept a `BigInt` source.)
- **Operators** — `+ - * / < > <= >= =` over `BigInt` reuse the SAME operator records; the units-style
  "dispatch on the resolved operand type" already exists (`apply_type` reads the operand `Ty`). A
  `BigInt` operand routes to the BigInt arithmetic (fold if constant, else emit a `bigint-*` op). A mixed
  `BigInt`/fixed operand is a `CDZ0301` mismatch from ordinary unification — no special rejection code.
- **`Prim`s** (resolved.rs): `BigIntOf`, `BigIntAdd/Sub/Mul/Div` (or route the generic `Add`… prims by
  operand type, mirroring how `Qty`/float arithmetic dispatch — PREFERRED, fewer prims), `BigIntConst`.
  Follow the "no keys outside the prelude" rule — every name is a prelude entry, dispatch on resolved
  `Prim`/`Ty`, never on `head == "…"`.

## §5 — Runtime `bigint.rs` (the `no_std` limb library)

A new `cdz-runtime/src/bigint.rs`, pure over `alloc::vec::Vec`, no I/O, unit-testable natively:
- **Representation in-module**: `struct Big { neg: bool, mag: Vec<u32> }` (base-2³² limbs, little-endian,
  no trailing zero limbs; zero = empty mag + `neg=false`). Canonicalization (`normalize`: strip trailing
  zero limbs, force `neg=false` on zero) after every op.
- **Algorithms** (schoolbook — correctness-first; magnitudes are small in practice):
  - `add_mag`/`sub_mag` (compare-and-subtract for signed add/sub), `cmp_mag`.
  - `mul` (O(n·m) schoolbook), `divmod` (Knuth Algorithm D, or simple long division — divmod is the one
    with real subtlety; test hard).
  - `gcd` (binary/Stein or Euclid — needed by `Rational` normalization).
  - `from_i64`/`to_i64_checked` (→ `Option`, for the checked narrowing), `from_le_two_complement_bytes`/
    `to_le_two_complement_bytes` (boundary), `from_sign_magnitude_bytes`/`to_sign_magnitude_bytes` (the
    heap-leaf form).
- **Heap glue in `lib.rs`**: `op_bigint_*` ops box a `Big` into a sign-magnitude `Raw` leaf (zero
  handles) via the Bytes-leaf pattern, and read it back. `op_dup`/`op_drop` need no change (raw-only
  leaf).
- **Tests**: differential against the compiler's `num-bigint` (native test can depend on it) over random
  operand pairs — add/sub/mul/divmod/gcd/cmp and both byte round-trips. This is the safety net for the
  hand-written arithmetic (analogous to the CHAMP-vs-BTreeMap reference oracle).

## §6 — Runtime WIT ops (appended, indices 63+)

Per the frozen-set contract, ops are APPENDED (never reordered). Adding each: edit `wit/runtime.wit`
(next index), add the `op_*` impl + `Guest` arm in `lib.rs`, run `cargo xtask codegen` (regenerates
`runtime_abi.rs` + re-derives `REQUIRED_RUNTIME_HASH` from the built bytes) and `codegen --check`. The
minimal op set (all params/results are lowerable `u32`/`s64`/`bool`):
- `bigint-of-i64 (s64) -> u32` — box a fixed-width int as a BigInt leaf (the `BigInt.of` target).
- `bigint-to-i64-checked (u32) -> ...` — the checked narrowing; traps out-of-range. (Trap vs. an
  Option-return is a design point — the spec says `(UInt 8).of (BigInt.of 300)` TRAPS, so trap.)
- `bigint-add / -sub / -mul / -div (u32 u32) -> u32` — the arithmetic; `-div` traps on zero divisor.
- `bigint-cmp (u32 u32) -> s64` — `-1/0/1` for `< = >` (comparison lowers to this + a fixed compare).
- `bigint-of-bytes (u32) -> u32` / `bigint-to-bytes (u32) -> u32` — boundary two's-complement `list<u8>`
  ↔ BigInt leaf (arg/result are the runtime's bytes-handle). Used only at an exported signature.

⚠ Only ADD ops that the compiler will actually EMIT. A constant-folded BigInt program (all 9 units cases)
emits NONE of the arithmetic ops — it needs at most `bigint-of-bytes`/`bigint-const` to materialize a
folded constant if it must cross to the runtime, and often not even that (a `Qty.value` of a constant
BigInt magnitude folds to the constant). See §9.

## §7 — `Rational` on top of `BigInt`

Once `BigInt` exists, `Rational` is a thin layer (the units cases need `Rational`, not `BigInt`
directly):
- **`Ty::Rational`** — a new nullary ground `Ty` (distinct type, no promotion — same discipline).
- **Representation**: a two-field value — numerator + denominator, each a BigInt — normalized (lowest
  terms via `gcd`, denominator strictly positive, sign on numerator). On the heap it is a small compound
  (two BigInt-leaf child handles) or a dedicated 2-handle node; canonical because always normalized.
- **`Rational` prelude module**: `of (n d)` → `Prim::RationalOf` (normalize immediately; **zero
  denominator traps** — "rational with zero denominator"), `of-int n` (`n/1`), `value`/`to-int`-style
  checked exits.
- **Arithmetic**: `+ - * /` over normalized pairs (cross-multiply + renormalize), exact; `/` by nonzero
  is total; comparison exact over the normalized pair. All reuse the BigInt ops on the two components.
- **Constant folding**: a constant `Rational` folds in the compiler (num-bigint pair + gcd) — so the 9
  units cases, whose Rationals are all constant, fold entirely at compile time.
- **Boundary**: `record { numerator: list<u8>, denominator: list<u8> }` (pinned) — but `Rational` is
  internal-only for the units cases (they `Qty.value` to a Rational that stays in-guest / is rendered);
  the boundary record is only needed if a `Rational` crosses an exported signature (later).

## §8 — Increment plan (each a landable slice; gate 0-fail per step)

- **B0 — `Ty::BigInt` through the closed universe (byte-neutral).** Add the `Ty` variant + every
  exhaustive-match arm + `encode_ty`/`decode_ty` round-trip + the `BigInt` prelude module (type-position
  only, no ops yet). Nothing constructs a BigInt → pass count UNCHANGED. Verify.
- **B1 — `BigInt.of` + constant folding + `Int64.of`-back (checked).** `Prim::BigIntOf`, `Core::
  ConstBigInt`, the checked narrowing extending `CheckedOf`. Constant `(BigInt.of 42)` folds; `(Int64.of
  (BigInt.of 42))` folds; out-of-range narrowing traps. NO runtime ops yet (all constant). This alone may
  clear several units cases if their Rationals are constant (they are).
- **B2 — `Ty::Rational` + `Rational.of`/`of-int` + constant folding + zero-denom trap.** The normalized
  pair over `num-bigint`; `Qty` becomes generic over `Rational` as inner (the units layer is ALREADY
  generic over `T` — `(Qty Rational u)` just needs `Rational` to be an admissible inner `Ty`). **This is
  the increment that flips the 9 units todos → pass** (their magnitudes are constant Rationals; the unit
  scale arithmetic is already exact). Constant Rational arithmetic (`+`/`/`/compare) folds in the
  compiler.
- **B3 — runtime `bigint.rs` limb library + the `bigint-*` WIT ops.** For RUNTIME-valued BigInts (a
  BigInt parameter, a loop accumulator). Differential-tested against num-bigint. Appends WIT ops,
  re-derives the runtime hash.
- **B4 — runtime `Rational` ops + boundary encodings.** Runtime-valued Rational arithmetic; the two's-
  complement `list<u8>` and rational-record boundary crossings.
- **B5 (optional, separate) — the `default-integer` pragma.** The module-directive channel + `CDZ0601/
  0602/0303`. Ergonomics, not needed for the units cases or the core types.

**Ordering rationale**: B0–B2 are PURE COMPILER work (num-bigint constant folding, no runtime change, no
hash change) and clear all 9 units cases. B3–B4 add the runtime arithmetic for non-constant BigInts,
which is where the hand-written `no_std` limb code and the frozen-hash re-derivation live. So the units
payoff (B2) lands WITHOUT touching the runtime at all — the runtime bignum (B3) is a separate, later
concern for programs that compute with runtime BigInt values.

## §9 — Key insight: the units cases need only B0–B2 (no runtime bignum)

All 9 units `todo` cases use CONSTANT Rational magnitudes (`(Rational.of 1 3)`, `(Rational.of 127 5000)`,
etc.) and CONSTANT unit scales. So they const-fold ENTIRELY in the compiler (num-bigint) and never reach
the runtime limb code. The runtime `bigint.rs` (B3) and runtime Rational (B4) are needed only for
programs with RUNTIME-valued big integers/rationals — a real capability, but NOT on the path to the units
cases. This means the units payoff is reachable through pure-compiler increments (B0–B2), and the
operator's "build bignum in the runtime" directive (B3) is the foundation for the general capability that
the units cases happen not to exercise. **Recommend landing B0–B2 first (clears the 9 cases, no runtime
risk), then B3–B4 for the general runtime capability.**

## §10 — Risks / watch-items

- **`encode_ty`/`decode_ty` silent-mis-encode** (the `Ty::Qty` lesson): a missing arm encodes `BigInt` as
  something else. Write both + a round-trip test FIRST.
- **Canonical byte form** is load-bearing: a BigInt/Rational used as a map key or `=`-compared must have
  ONE representation. Normalize on every construct (strip trailing zero limbs; Rational to lowest terms +
  sign on numerator). Test that two constructions of the same value are `champ_eq`/hash-equal.
- **divmod correctness** is the one genuinely tricky algorithm — differential-test hard against
  num-bigint over random (including near-limb-boundary) operands.
- **Frozen runtime hash** (B3+): every runtime edit re-derives `REQUIRED_RUNTIME_HASH`; run `cargo xtask
  codegen` (never hand-edit) + `codegen --check`; build with `cargo component build`; a comment edit in
  `cdz-runtime` also bumps the hash.
- **No silent promotion**: `agrees_with` must be TRUE only `BigInt`↔`BigInt` (and `Rational`↔`Rational`),
  so a mixed operand is `CDZ0301`. Do NOT add any `join`/coercion that unifies `BigInt` with a fixed
  width.
- **The seed's fixnum window is 30-bit, boxed-int is i64**: a `BigInt` that happens to fit i64 is STILL a
  distinct `BigInt` value with its own leaf — do NOT collapse it to `imm_int`/boxed-Int64 (that would
  conflate two distinct types). Its canonical form is the sign-magnitude leaf regardless of magnitude
  (except the compiler may keep small constants folded).

## §11 — Summary

`BigInt` is a new distinct nullary ground type (`Ty::BigInt`), represented at runtime as a sign-magnitude
limb-array leaf (raw bytes, zero handles — the Bytes-leaf shape) and at compile time as a
`num-bigint`-folded constant. `Rational` is a normalized BigInt pair on top. The 9 units cases are all
constant and clear at the pure-compiler layer (B0–B2) with no runtime change; the hand-written `no_std`
runtime limb library (B3) provides the general runtime capability the operator asked for, differential-
tested against num-bigint. Boundary crossings use the pinned two's-complement `list<u8>` (BigInt) and
`{num,den}` record (Rational). Every step is a landable, 0-fail gate slice.
