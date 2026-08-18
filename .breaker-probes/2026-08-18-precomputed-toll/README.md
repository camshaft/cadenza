# Toll computed before the suspend, consumed after (2026-08-18)

- `pyw1.sexp` — (let ((t (* 100 (+ s 1)))) (+ (resume s (+ s 2)) t)):
  the toll is LET-BOUND from pre-resume state, then the resume happens,
  then the SAVED binding is consumed post-replay. The binding must ride
  the continuation across suspend + replay (631 = fold 31 + t2 300 + t1
  200). Distinguishes recompute-from-post-state (would use s+2) and
  dropped-slot lowerings. The dual of pyv1 (op-arg capture) with a
  DERIVED local instead — and the let sits AROUND the resume rather than
  binding it (so neither binder fix's surface). Exercises the newly
  refactored refold_let_by_binder_inline path from the pre-resume side.
  PASS x3 at 6acb06588.
- `pyw2.sexp` — TWO pre-suspend bindings ride together: t1 (x100, s+1)
  and t2 (x10, s+2) both let-bound before the resume, both summed after
  the replay (831 / 600, CPS-modeled). Dropping or recomputing either
  slot shifts a distinct digit range. Extends pyw1 (one slot) to the
  multi-slot continuation frame. PASS x3 at 942944f3f.
