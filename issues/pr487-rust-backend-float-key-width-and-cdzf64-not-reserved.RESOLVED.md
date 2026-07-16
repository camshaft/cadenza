# PR #487 (merged, batch 116) — rust-backend Float Set/Map key: width-blind CdzF64 wrap + CdzF64 collides with user type

Mirrored from Copilot inline on merged PR #487 (2 comments). Confirmed on trunk.
Owner: **v-rust-backend** (bare-Float Set/Map-key support this batch).

## 1. ord_key_type wraps ALL floats as CdzF64, ignoring width (comment 3596987794, rust/types.rs:190)
> `ord_key_type` treats every `Ty::Float(_)` as `CdzF64`, but `rust_type` correctly distinguishes
> Float32→`f32` vs Float64→`f64`. A `Set Float32` / `Map Float32 _` will emit `BTreeSet<CdzF64>` /
> `BTreeMap<CdzF64,_>` and `CdzF64::new(<f32 expr>)`, which will not type-check (and would be
> semantically wrong even if coerced). Needs a width-specific wrapper (`CdzF32` + `CdzF64`) or an
> equivalent width-aware key strategy, and the key wrap/unwrap paths must agree with it.

Trunk `rust/types.rs:188`: `Ty::Float(_) => Some("CdzF64".to_string())` — matches ANY float width.
`rust_type` (same file, ~line 32) correctly emits `f32`/`f64` by `Ty::Float(ft)` width. So a
Float32-keyed Set/Map emits a CdzF64 wrapper around an `f32` expr → rustc type error. Real miscompile
for the narrow-float key case (the bare-Float64 key the corpus exercises works; Float32 does not).

## 2. CdzF64 wrapper name is not reserved against user code (comment 3596987844, rust/mod.rs:174)
> The `CdzF64` wrapper insertion is gated by `out.contains("CdzF64")` and the name is not actually
> reserved. A user can define a sum type named `CdzF64` (a valid Cadenza name; `sum_ident`/
> `sanitize_ident` emit it unchanged), producing both `enum CdzF64 {…}` and the injected
> `struct CdzF64(u64);` → rustc duplicate-definition (E0428), even for programs that never use float
> keys.

Trunk `rust/mod.rs`: `if out.contains("CdzF64")` with a comment asserting "Cdz-prefixed, never a user
ident, so a substring match cannot false-positive" — but `CdzF64` IS a legal user sum-type name, so
the assertion is false. Suggested: (a) a genuinely backend-reserved scheme (`__cdz_*` namespace that
user idents are mangled away from, like `sum_ident`'s marker); (b) gate injection on an unambiguous
usage marker (`CdzF64::new(`) rather than a raw substring.

## Note
These interact: fixing #1 with a `CdzF32` wrapper widens the reserved-name surface #2 must cover.

PR: https://github.com/camshaft/cadenza/pull/487
