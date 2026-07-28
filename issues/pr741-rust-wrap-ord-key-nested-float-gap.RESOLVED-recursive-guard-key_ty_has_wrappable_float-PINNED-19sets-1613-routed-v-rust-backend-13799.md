# PR#741 review comment — rust wrap_ord_key RECORD/TUPLE key: partial-move risk + nested-float gap

Mirrored from GitHub PR review comment (Copilot), id `3623377025`.
PR: https://github.com/camshaft/cadenza/pull/741 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:256` (the `Ty::Record` branch of `wrap_ord_key`, ~213-258)

## Comment (verbatim)

> `wrap_ord_key`'s new `Ty::Record` branch rebuilds the key by referencing tuple fields
> (`__k.0`, `__k.1`, …). This triggers Rust partial-move errors when an earlier field is
> non-`Copy` (e.g. `String`) and a later field is accessed, which is especially likely because
> record fields are in sorted-name order (e.g. `(record (a "…") (f 1.0))`). Also, gating the
> rebuild on "any direct float field" can miss cases where a field's type contains a float that
> needs wrapping (e.g. a tuple field containing a float), leading to a key-value/type mismatch in
> emitted Rust.

## Liaison verification (PARTIAL — needs owner repro)

`wrap_ord_key` (expr.rs:213), landed by v-rust-backend `94ea8c58b` ("thread the float-key
ord-wrapper through a RECORD key", extends the tuple arm `d0d18e257`). Two distinct concerns:

1. **Nested-float gap — LOOKS REAL.** Both the `Ty::Tuple` (~220) and `Ty::Record` (~240) arms gate
   the rebuild on `.any(|e|/|t| matches!(…strip_nominal(), Ty::Float(_)))` — a DIRECT float
   element/field only. A key whose field/element TYPE *contains* a float but isn't itself a float
   (e.g. a record field of type `(Tuple Float …)`, or a nested record) does NOT trigger the rebuild,
   so that nested float is never wrapped in `__CdzF32/F64` → the emitted key type won't match the
   `__Cdz*`-wrapped value type. (The per-element recursion `wrap_ord_key("__k.{i}", e)` DOES handle
   nesting correctly ONCE entered — the bug is the *guard* that decides whether to enter.)
   Consider a recursive "contains a float leaf" predicate for the guard instead of a shallow `.any`.

2. **Partial-move on non-Copy earlier field — UNCONFIRMED (may be a false alarm).** The rebuild binds
   `let __k = <expr>;` then builds `(__k.0, __k.1, …)`. Rust *does* allow disjoint partial moves out of
   a local `__k` (each `__k.i` moved into a distinct tuple slot, no field used twice), so a `String`
   earlier field + later field access may actually compile. BUT if any `wrap_ord_key("__k.{i}", …)`
   path BORROWS `__k.i` (e.g. `__CdzF64::new(__k.1)` takes by value while `__k.0` is a moved String)
   the borrow/move interaction could still fault. I could NOT confirm this without emitting + `cargo
   check`-ing a concrete case — owner should repro with e.g. `(record (a String) (f Float64))` as a
   Map key and inspect the generated Rust.

Owner: v-rust-backend (`94ea8c58b`). Routed as a note flagged POTENTIAL-CORRECTNESS (invalid/mismatched
emitted Rust) — worth a concrete repro: build a Map keyed by a record with (a) a nested-float field and
(b) a non-Copy-then-float field, emit rust, `cargo check` the output. If it reproduces, add the repro as
a rust-backend regression pin.
