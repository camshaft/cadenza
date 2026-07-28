# PR#695 review comment — effects.rs doc comment attached to wrong function

Mirrored from GitHub PR review comment (Copilot), id `3616095825`.
PR: https://github.com/camshaft/cadenza/pull/695
Location: `implementation/seed/crates/rcdzc/src/effects.rs:4827`

## Comment (verbatim)

> The doc comment starting here describes `body_calls_def` (residual self-call check),
> but it's placed immediately before `fold_ctor_match_through_lets`, so Rust will attach
> it to the wrong function. This makes the docs misleading and leaves `body_calls_def`
> undocumented. Please split/move the comment so each function's doc matches its behavior
> (e.g., keep the `body_calls_def` description immediately above `fn body_calls_def`, and
> keep only the let-wrapped match-folding description above `fold_ctor_match_through_lets`).

## Liaison verification (CONFIRMED)

Read the source on trunk:
- Lines ~4823–4828: doc paragraph describing the *residual self-call check* (i.e. what
  `body_calls_def` does) — "Whether `node` contains an APPLICATION whose head resolves to `def`…".
- Line 4834: `fn fold_ctor_match_through_lets(...)` — the fold-through-lets doc paragraph is
  ALSO present just above it (correct), but the self-call paragraph precedes both, so Rust
  glues the self-call doc onto `fold_ctor_match_through_lets`.
- Line 4879: `fn body_calls_def(...)` — currently UNDOCUMENTED.

Fix: move the residual-self-call doc paragraph down to immediately above `fn body_calls_def`
(4879); leave only the let-wrapped match-folding paragraph above `fold_ctor_match_through_lets`.

Doc-only, no behavior change. Routed to v-effects (owner of `rcdzc/src/effects.rs`).
