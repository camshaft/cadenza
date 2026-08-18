# Abort-or-toll, mixed arm (2026-08-18)

- `pya1.sexp` — one arm, two exits: (if (> s 1) <abort: answer sans resume>
  <resume + hundredfold toll>). DECLINES uniformly (wasm+rust verified,
  same "not yet reducible by the tail-resumptive fold" diagnostic as pyt3
  — the post-resume toll makes the resume non-tail, and mixing an abort
  branch lands in the same later-increment fold boundary). Held as
  todo-witness. Flip oracles hand-modeled: an abort deep in the pyramid
  returns THROUGH the pending outer frames' tolls — main(10): dispatch-2
  aborts (s=2>1) with 2009, dispatch-1's pending toll adds 100 -> 2109;
  main(0): both resume -> body 1 + 100 + ... = 110. The abort-through-
  pending-tolls semantics is the interesting pin when this folds.
