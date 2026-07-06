# Type Mapping — Choice: component-model-types

> **The default choice for the `type-mapping` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete Cadenza-to-host-interface
> type table that realizes the component-abi.md frozen contract's requirement that each Cadenza type
> appearing in an exported or imported signature has a single stable representation in the host
> interface's type system.
>
> This is an ABI-level choice: a change to an existing row alters bytes produced from unchanged source
> and is therefore a coordinated ABI change under the constitution's Governance Floors. Adding a row
> for a type that previously had no boundary representation is an additive change.

## The table (Cadenza type → component-model / WIT type)

| Cadenza type | Boundary representation |
|---|---|
| Unit | the empty result payload (`result<_, …>` success, or an empty `tuple`/`record {}`), carrying no bytes |
| Boolean | `bool` |
| Signed integer (8/16/32/64-bit) | `s8` / `s16` / `s32` / `s64` |
| Unsigned integer (8/16/32/64-bit) | `u8` / `u16` / `u32` / `u64` |
| Floating-point (binary64) | `f64` |
| Big-integer | `list<u8>` in a fixed canonical two's-complement encoding |
| Rational | `record { numerator: list<u8>, denominator: list<u8> }`, normalized |
| `Char` (a validated Unicode scalar) | `char` |
| String (UTF-8) | `string` |
| List of `T` | `list<T'>` where `T'` is the mapping of `T` |
| Set of `T` | `list<T'>` in canonical element-sorted order (the same fixed element order the set's canonical byte form and iteration use) |
| Tuple / structural record | `record { … }` with fields in canonical order |
| Nominal struct | `record { … }` named by its declared name |
| Sum type / enum | `variant { … }` |
| Optional `T` | `option<T'>` |
| Result of `T` or `E` | `result<T', E'>` |
| `Never` (the empty sum) | no boundary representation — `Never` is uninhabited, so no value of it ever crosses the boundary |
| Function | not directly representable; exposed as a resource handle where a boundary-crossing function is required |

## Notes

- **Field ordering is canonical**, derived from the declared type, not from discovery order, so the
  `record` layout is determined by the type alone (component-abi.md §"Aggregate Layout Is Determined
  By Type").
- **Generics are monomorphized** before this table applies; no generic type appears at the boundary
  (component-abi.md §"Generics Do Not Cross The Boundary").
- **Functions** do not cross the boundary by value; a program that must expose a callback exposes it
  as a resource, keeping the boundary first-order.
- The big-integer and rational encodings are fixed here so that an exact numeric value has one
  boundary form regardless of how it was computed.
- **`Char`** is the surface `Char` type — a validated Unicode scalar (collections-and-text.md §"A Char
  Is A Single Unicode Scalar Value") — realizing this `char` row; the row is not an orphan the language
  cannot produce (see `spec/learnings/2026-07-05-char-is-a-validated-unicode-scalar-the-boundary-already-promises.md`).
- **`Set`** serializes as a `list<T'>` whose elements are in the fixed canonical order derived from the
  elements (deterministic-value-form.md §"Ordering Of Aggregate Members Is Fixed" — a set is an
  unordered aggregate), the same order set iteration visits (collections-and-text.md §"Set Iteration Is
  Deterministic"), so a set has one boundary form regardless of insertion order — exactly as a map's
  keys already do. Adding this row is additive (a value that previously had no boundary representation).
- **`Never`** never appears at the boundary: it is the empty sum (type-system.md §"Never Is The Empty
  Sum"), uninhabited, so no value of it is ever produced to cross — the table lists it only to record
  that its absence is intentional, not an omission.
