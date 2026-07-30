# PR#909 review comment — 15-rows Record.merge-closure doc misattributes 87/37 to `b` (corpus-bugfix)

Mirrored from GitHub PR#909 review comment (Copilot), id `3679786796`.
File: `spec/semantics/15-rows-and-open-sums.sexp:1222` — corpus doc → corpus-bugfix. Blame `a4524bee4`
"corpus(rows): 4-pin drain K — closure fields through the row-op family".

## Comment (verbatim)

- (id 3679786796, 15-rows-and-open-sums.sexp:1222) "In this doc string, 'beside the merged `b` (87 at
  k=5, 37 at k=0)' is inaccurate: `b` is always 7; 87/37 are the *overall expression* results. This
  makes the description misleading for readers trying to understand what the case is asserting."

## Liaison verification (confirmed on trunk 9c77673b0)

Case "Record.merge carries a CLOSURE field into the union layout and it applies" (:1220). Body:
`(def m (Record.merge (record (f (fn ((: y Int64)) (+ y k)))) (record (b 7)))) (+ (* 10 ((. m f) 3)) (. m
b))`. So `(. m b)` is ALWAYS 7 (the merged scalar record `(b 7)`), and the results are `10 * (3+k) + 7`
= 87 at k=5 (`10*8+7`) and 37 at k=0 (`10*3+7`) — matching the `(output …)` pins. The doc says "the
MERGED record's `f` applies (3+k → ×10) beside the merged `b` (87 at k=5, 37 at k=0)". Parenthesizing
"87 at k=5, 37 at k=0" right after "`b`" reads as `b`'s value, but those are the WHOLE-expression outputs;
`b` is the constant 7. Misleading. Fix: move the 87/37 to describe the overall result (e.g. "…applies
(3+k → ×10), plus the merged `b`=7 → 87 at k=5, 37 at k=0"). Doc-only, pins correct.

Owner: **corpus-bugfix** (`spec/semantics/15-rows-and-open-sums.sexp`; `a4524bee4`). Reword to attribute
87/37 to the overall result, not `b`.
