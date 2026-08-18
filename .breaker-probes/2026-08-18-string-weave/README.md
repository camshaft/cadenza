# String state through resumes; heap-typed post-resume boundary (2026-08-18)

- `pys2.sexp` — STRING state thread growing through tail resumes:
  (resume s (String.concat s "x")), seed picks ropes of different LENGTHS
  ("a" vs "bb") so byte-len separates runs (5 / 3). PASS x3.

Boundary finding (ladder in /tmp, todo-witnesses not banked — same
fold-boundary class as pyt3/pya1): POST-RESUME expressions with
HEAP-typed (String) operands decline at the tail-resumptive fold —
(String.concat (resume ...) s) declines, and even an Int64-result
(String.byte-len (String.concat "ab" (resume ...))) declines. So the
post-resume fold surface is currently SCALAR-only: every passing pyr/pyt
/pyw shape is Int64. The heap-typed post-resume face joins the pyt3/pya1
later-increment watch (not filed — same documented diagnostic).
