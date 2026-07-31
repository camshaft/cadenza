# PR#955 + PR#956 review comments — two corpus follow-on nits (corpus-bugfix)

Two Copilot review comments, both `spec/semantics/*.sexp` → corpus-bugfix.

## Comment 1 (verbatim) — PR#955, 07-type-system:1882 (phrasing follow-on to PR#952)

- (id 3692588456, 07-type-system.sexp:1882) "The updated docstring resolves the contradiction, but the
  phrase 'as this case adds' reads like the test case is changing the language rather than documenting
  behavior. Consider wording this as a plain description of the operand family (including PERFORM
  results) for clarity and consistency with other corpus docs."

### Liaison verification (confirmed on trunk 5a291af00; blame `5fdc9276f`)

The PR#952 fix (`5fdc9276f` "Type.of operand family INCLUDES the perform this case adds") resolved the
"never a PERFORM" contradiction — the doc now reads "…covers literals/params/constructions/generic sums,
and — **as this case adds** — a PERFORM result: …". Copilot's meta-point: "as this case adds" frames the
TEST as changing the LANGUAGE, when a corpus doc should plainly DESCRIBE the behavior. Fair, and it used
the liaison's own suggested phrasing from the PR#952 route (my wording — corpus-bugfix took it verbatim).
Reword to a plain operand-family description, e.g. "…covers literals, params, constructions, generic
sums, and PERFORM results (the reflected type is the op's DECLARED result type…)". Doc-only, pin correct.

## Comment 2 (verbatim) — PR#956, 18-units:3285 (witness doesn't match stated intent)

- (id 3692679636, 18-units-of-measure.sexp:3285) "The PR description mentions adding an INPUT-position
  derived-unit annotation witness for `meter / second ^ 2`, but this new case currently uses `meter ^ 2`.
  If the intent is to pin parsing/precedence for `/` combined with an exponent (as described), switching
  the annotated unit here to `meter / second ^ 2` would better match the stated goal."

### Liaison verification (confirmed on trunk 5a291af00; blame `5a291af00`)

Case "a derived-unit type annotation in INPUT position round-trips and the param computes". The annotated
unit is `(Qty Int64 (Unit.^ (Unit.base #"meter") 2))` = `meter^2` — a bare EXPONENT, NO division. The doc
says it's "the corpus witness for the type_ref infix fix" and exercises "print→re-parse AND the annotated
param computing". If the stated intent (per the PR description) was to pin `/`-combined-with-exponent
precedence (`meter / second ^ 2`), the current `meter^2` under-covers — it exercises the exponent infix
but NOT the `/`-vs-`^` precedence interaction. Fix (Copilot's, per stated intent): switch the annotated
unit to `meter / second ^ 2` (a `Unit./` of meter and `Unit.^ second 2`) so it pins the division+exponent
precedence the description promised, while still round-tripping + computing. (Owner confirms the intended
scope — if `meter^2` was the deliberate minimal witness and the `/` case is a separate follow-up, a doc
tweak instead; but the description↔case mismatch should be reconciled either way.) Corpus coverage.

Owner: **corpus-bugfix** (both `spec/semantics/*.sexp`; `5fdc9276f` + `5a291af00`). Reword the Type.of doc
phrasing; reconcile the derived-unit witness with its stated `meter / second ^ 2` intent.
