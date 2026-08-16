# Site-6 x nested same-effect re-handle (2026-08-10)

Angle: the Site-6 through-block float (ff76dd2e5) peels pure wrappers off a
branch-performing conditional. What if the SAME effect is re-handled around/inside
the wrapper? The float must attribute performs to the right handler.

GREEN x3 (pin candidates):
- n1: block-wrapped branch-perform INSIDE an inner re-handle of the same effect —
  inner strides +10, outer +1; float attributes the perform to the inner handler — 1010034
- n2: wrapped conditional's ELSE leg performs (then leg pure) inside the inner
  re-handle — both legs exercised — 75004/220705

DECLINE WITNESS (honest floor, NOT a bug — banked for v-effects' Site-6 follow-ons):
- n3: the wrapper binding's init is itself a DISCHARGED inner same-effect handle
  (performs inside cannot escape). With a LITERAL inner seed + const-foldable condition
  the whole inner handle folds away and the case passes; with a dynamic condition
  (`(= (+ w n) 65)`) the wrapped conditional survives and Site-6's pure-peel consults
  reaches_any_perform, which counts the discharged performs -> wrapper "impure" ->
  decline ("not yet reducible by the tail-resumptive fold"). A discharged handle is
  semantically pure from outside; discounting it is a possible later increment.
