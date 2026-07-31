# PR#952 review comment — 07-type-system Type.of-over-perform doc says "never a PERFORM" contradicting the case (corpus-bugfix)

Mirrored from GitHub PR#952 review comment (Copilot), id `3692209771`.
File: `spec/semantics/07-type-system.sexp:1882` — corpus doc → corpus-bugfix. Blame `ea05e923d`
"corpus(3 files): 3-pin drain AK — Type.of over a perform, …".

## Comment (verbatim)

- (id 3692209771, 07-type-system.sexp:1882) "The case docstring says Type.of's operand family is 'never a
  PERFORM', but this new case is explicitly validating Type.of over a PERFORM result. That wording is now
  self-contradictory and could mislead readers about the current behavior being pinned by the corpus."

## Liaison verification (confirmed on trunk 0fb03f4c1)

Case TITLE: "Type.of over a PERFORM result reflects the op's declared result type". Its `(input …)` is
`(Type.eq (Type.of (E.get)) (Type.of (list 1 2)))` — `(E.get)` IS a perform, so the case is explicitly
pinning Type.of OVER a perform. But the DOC opens: "The Type.of operand family covers
literals/params/constructions/generic sums — **never a PERFORM**: the reflected type is the op's DECLARED
result type…". So the doc says perform is NOT in the Type.of operand family while the case demonstrates
exactly that. Self-contradictory (likely lifted from an EARLIER Type.of case's doc that predated this
perform extension). Fix: reword the doc's operand-family clause to INCLUDE the perform case this pin adds
(e.g. "…covers literals/params/constructions/generic sums, AND — as this case adds — a PERFORM result,
whose reflected type is the op's DECLARED result type resolved through the handler frame"). Doc-only, pin
correct (1).

Owner: **corpus-bugfix** (`spec/semantics/07-type-system.sexp`; `ea05e923d`). Reword the "never a PERFORM"
clause to match this case pinning Type.of over a perform.
