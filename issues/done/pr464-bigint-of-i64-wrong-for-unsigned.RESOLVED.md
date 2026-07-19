# PR review comment — mirrored from GitHub PR #464 (Copilot inline)

- **PR:** #464 "fleet: batch 94 (…, rust-backend BigInt emit-side, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:1491` (`Core::BigIntOfI64`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3594490672
- **Link:** https://github.com/camshaft/cadenza/pull/464#discussion_r3594490672

## Comment (verbatim)
> `Core::BigIntOfI64` currently emits `cdz_num::Big::from_i64(({v}) as i64)`. This is incorrect for unsigned integer operands (e.g. `UInt64`), because casting `u64` to `i64` will reinterpret values ≥ 2^63 as negative, producing the wrong `BigInt.of` result. Since `BigInt.of` is typed as `∀a. (Int a) -> BigInt` (covers unsigned widths too), the Rust backend should widen unsigned operands by value (e.g. via sign-magnitude bytes from a `u64`) rather than an `as i64` cast.

## Liaison triage — CONFIRMED against trunk
Confirmed in expr.rs: `Core::BigIntOfI64 { value } => Ok(format!("cdz_num::Big::from_i64(({v}) as i64)"))`.
`BigInt.of` is `∀a. (Int a) -> BigInt`, so its operand can be an UNSIGNED width (UInt64). A `u64` value
≥ 2^63 cast `as i64` reinterprets as NEGATIVE → `Big::from_i64` builds the wrong (negative) BigInt →
`BigInt.of(u64_big_value)` is silently wrong. New rust-backend BigInt emit-side (this batch). FIX: widen
unsigned operands BY VALUE (e.g. sign-magnitude bytes from the `u64`, or a `Big::from_u64` path) instead
of the blanket `as i64` cast; only signed widths use `from_i64`. v-rust-backend. Fix on `trunk`. Quote +
link in queue file.
