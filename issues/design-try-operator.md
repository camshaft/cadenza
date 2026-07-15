# Vertical-ready: the `?` / `try` fallible short-circuit operator

**Design doc:** `implementation/design/DESIGN-try-operator-rcdzc.md` (landed on trunk).
**Subsystem:** `rcdzc` (front-half: surface + resolve + infer + Hir→Mir desugar). NO backend/runtime change.
**Coordinates with:** `v-syntax` (surface + round-trip, owns T0 surface half), `v-inference` (the
bidirectional boundary-type check), `v-diagnostics` (CDZ0230 + fix hints). `v-effects` for context only —
`?` reuses the E4 abortive `Mir::Block`/`Break` substrate but adds no user-visible effect.

**The crux, decided:** NOT a monad wedge. `?` desugars onto the effects system's within-function abortive
lowering; zero new abstraction; contributes nothing to the effect row/manifest. Consistent with the
traits-are-dictionaries decision to defer HKT/Monad.

**First increment (T1 is the core win, gated behind T0):**
- **T0 — surface + type + rejections (no lowering).** ML postfix `parse(a)?`; canonical s-expr `(try e)`;
  binary `Try` node; round-trip on all three surfaces. `Try` Hir leaf carried through resolve/infer; the
  bidirectional boundary-type check; **CDZ0230** (a `?` with no fallible boundary) + the CDZ0203 mismatch.
  Green = reject cases + pure type-check cases.
- **T1 — function-boundary lowering.** Synthesize the boundary `Mir::Block` around a function body with `?`;
  desugar `e?` to `match … | short => Mir::Break` for both Result and Option; a value EXECUTES through
  wasmtime (happy + short-circuit path). Delivers the operator's ask.
- T2 — `try { }` block boundary (v2). T3 — prelude `Result.map-err`/`Option.ok-or` conversion idiom.

**Corpus:** new `spec/semantics/22-try-operator.sexp`.
**Gate:** `cargo test -p rcdzc --lib` (desugar unit + wasmtime run, both paths + a CDZ0230 reject),
`cargo xtask gate` (additive fail-set), `cargo xtask check`. The executing wasmtime cases are the ones that
matter — `?` is control, a value must come out the far side.
