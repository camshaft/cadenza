# Bool-negating post-resume arm under a short-circuit body (2026-08-18)

- `pyb1.sexp` — the arm NEGATES the rest-of-body value: (not (resume (> s
  0) (+ s 2))); the or-body's short circuit decides HOW MANY negating
  frames stack. Seed>0: first draw answers true, or short-circuits, ONE
  negation -> false -> 2. Seed=0: two draws, TWO negations restore ->
  true -> 1. Parity-of-frame-count as the observable. Post-resume surface
  note: Bool negation works with a Bool-valued BODY, but the same arm
  over an Int64-result body (if (or ...) 1 2 INSIDE the handle) declines
  at the fold boundary — the if must wrap OUTSIDE the handle. Also: the
  Int64-answer variant with two separate probes declines. The scalar-only
  boundary (pys2 README) refines to: the handled BODY's type and the
  post-resume expression must agree... (ladder in /tmp). PASS x3 at
  e4bf6e301.
- `pyb2.sexp` — the parity law at three probes: each draw answers state
  evenness, the nested and-chain short-circuits at the FIRST odd state.
  Seed 1 (n=10): first draw odd -> false, ONE frame, not(false)=true ->
  1. Seed 0: draw even -> continue; draw odd -> short-circuit false,
  TWO frames, not(not(false))=false -> 2. Frame count is data-dependent
  through a short-circuit chain rather than fixed by the body shape.
  PASS x3 at e4bf6e301.
