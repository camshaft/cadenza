# Type Mapping — Declared Default

> **What this file is.** The concrete Cadenza-to-host-interface type table that realizes the
> component-abi.md frozen contract's requirement that "each Cadenza type that may appear in an
> exported or imported signature has a single stable representation in the host interface's type
> system." The contract states the property technology-neutrally; this file pins the table.
>
> This is a **declared default** at the ABI level: a change to an existing row alters bytes produced
> from unchanged source and is therefore a coordinated ABI change under the constitution's
> Governance Floors. Adding a row for a type that previously had no boundary representation is an
> additive change.

## The table (Cadenza type → component-model / WIT type)

| Cadenza type | Boundary representation |
|---|---|
| Boolean | `bool` |
| Signed integer (8/16/32/64-bit) | `s8` / `s16` / `s32` / `s64` |
| Unsigned integer (8/16/32/64-bit) | `u8` / `u16` / `u32` / `u64` |
| Floating-point (binary64) | `f64` |
| Big-integer | `list<u8>` in a fixed canonical two's-complement encoding |
| Rational | `record { numerator: list<u8>, denominator: list<u8> }`, normalized |
| Character (Unicode scalar) | `char` |
| String (UTF-8) | `string` |
| List of `T` | `list<T'>` where `T'` is the mapping of `T` |
| Tuple / structural record | `record { … }` with fields in canonical order |
| Nominal struct | `record { … }` named by its declared name |
| Sum type / enum | `variant { … }` |
| Optional `T` | `option<T'>` |
| Result of `T` or `E` | `result<T', E'>` |
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
