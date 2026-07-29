# PR#890 review comment — emit_sum_cmp_walk expands inline (no seen/helper guard) → unbounded codegen on a recursive Option-carrying sum (⚠ v-rust-backend)

Mirrored from GitHub PR#890 review comment (Copilot), id `3671882723`.
File: `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:3436` — v-rust-backend. Blame
`7392dc3b8` "rcdzc(rust): SOUNDNESS #42 — order Option by declared Some<None, not std's None<Some".

⚠ CORRECTNESS (codegen non-termination) — flagged for v-rust-backend.

## Comment (verbatim)

- (id 3671882723, backend/rust/expr.rs:3436) "`emit_sum_cmp_walk` is documented as being routed through
  a helper for recursive sums, but it currently expands inline and does not use the `seen` recursion
  guard / `helpers` sink. For a recursive sum whose payload contains a built-in `Option` (so
  `ty_uses_flip_order_option` is true), this can trigger unbounded recursion in codegen (stack overflow /
  runaway generated Rust) when the sum reappears in its own payload."

## Liaison verification (confirmed on trunk 9872e4458)

Doc (expr.rs:3441-3442): "Routed through a helper `fn __cmp_<Ident>` for a recursive sum (like
`emit_value_eq_walk`'s `__eq_` helper) so it terminates." But the BODY (3445-3499) expands INLINE:
- it takes `seen: &mut Vec<StructId>` + `helpers: &mut Vec<String>` params, but NEVER checks `seen`
  (no "already-emitting this decl → emit a helper call instead" guard) and NEVER pushes a `__cmp_<Ident>`
  fn into `helpers`.
- the same-variant payload arm (3484) recurses via `emit_value_cmp_walk_seen(db, &payload_ty, …, seen,
  helpers)` — threading `seen`/`helpers` DOWN, but the sum arm of that walk (3423) just calls
  `emit_sum_cmp_walk` again, which again expands inline.
So for a recursive sum `T` whose payload (transitively) contains a built-in `Option` — which forces the
`ty_uses_flip_order_option` route into `emit_sum_cmp_walk` instead of the native `.cmp()` early-return —
when `T` reappears in its own payload the inline expansion recurses without a `seen` cutoff → unbounded
codegen (compiler stack overflow or runaway generated Rust). The `emit_value_eq_walk` sibling (its `__eq_`
helper) is the DONE pattern the doc claims but the cmp walk doesn't follow.

Witness shape (owner to confirm): `type T = (Node (Tuple (Option Int64) T)) | (Leaf)` (a recursive sum
whose payload carries an `Option`), then a value-cmp of two `T`s (e.g. `List<T>` sort, or `T` as a
Set/Map key on the rust target) — should generate a terminating `__cmp_T` helper; per Copilot it currently
inline-recurses. Fix: mirror `emit_value_eq_walk`'s helper routing — on entry, if `decl_occ` is already in
`seen`, emit a `__cmp_<Ident>(l, r)` CALL and push the `fn __cmp_<Ident>` body into `helpers` once, so the
recursion goes through a named fn and terminates.

Owner: **v-rust-backend** (`backend/rust/expr.rs` value-cmp; `7392dc3b8` SOUNDNESS #42). Route through a
`__cmp_` helper with the `seen` guard, matching the eq-walk; add a recursive-Option-sum cmp witness.
