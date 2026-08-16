# Closure-captures-draw x effects (2026-08-10)

Angle: a closure capturing an effect draw must freeze the captured VALUE — later
draws/state advances must not leak into the capture, and captures must survive
higher-order boundaries.

GREEN x3 (pin candidates):
- cc1: closure captures draw d1, invoked AFTER a later draw — capture stays d1 — 50304/20001
- cc4: captured draw crosses a HIGHER-ORDER def boundary (helper applies the closure
  twice); capture fixed while call args vary — 403321/100021
  (authoring slip: my in-file pins disagreed with the python model; compiler agreed
  with python — expectations corrected, THEN green. The model is the oracle, not my head.)

DECLINE WITNESSES (staged, honest):
- cc2: closure BODY performs, invoked twice — "performed with no enclosing handler
  here; its home is determined by the handler or delegation enclosing its callers"
  (the closure's perform-home analysis doesn't yet thread through let-bound fn values)
- cc3: closure BUILT in a performing BRANCH (each branch draws then closes over it) —
  "not yet reducible by the tail-resumptive fold"

Vocab: lambda form is `(fn (k) body)` — `lambda` is not a binder (CDZ0101 unbound).
