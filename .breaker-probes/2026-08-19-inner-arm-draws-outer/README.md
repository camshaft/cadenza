# inner-arm-draws-outer — inner handler's arm draws from the OUTER effect (legal enclosure)
## pyid1 — I.now arm resumes (+ t (O.get)); O is the outer handler. Model 73062/63052. PASS x3.
Two handler states (O, I) thread independently; each inner answer folds in an outer sub-draw at
the outer's current state. Well-typed (outer encloses inner). Promotable pass-witness.

## Side note (pyie1, NOT banked as probe): intra-EFFECT self-dispatch in the answer
An arm performing a DIFFERENT op of its OWN effect in the resume answer (E.tick's arm doing
(E.aux)) is CDZ0401-rejected ("reached with neither an enclosing handler nor host delegation") —
the arm runs outside its own handler, so re-performing its effect there has no home. Well-formedness
reject, not a fold issue; not a finding. (Same rule as the self-recursive-arm reject from tick 1901.)
