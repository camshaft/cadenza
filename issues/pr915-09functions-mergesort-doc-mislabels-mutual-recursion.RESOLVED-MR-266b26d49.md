# PR#915 review comment — 09-functions merge-sort doc mislabels "mutual recursion" (no cycle) (corpus-bugfix)

Mirrored from GitHub PR#915 review comment (Copilot), id `3681654830`.
File: `spec/semantics/09-functions.sexp:6544` — corpus doc → corpus-bugfix. Blame `1f526f4bf`
"corpus(functions): 2-pin drain O — merge sort (recursion tree) + Ackermann (nested recursion)".

## Comment (verbatim)

- (id 3681654830, 09-functions.sexp:6544) "The doc string says this is 'mutual recursion
  msort->msplit…->merge…', but there is no mutual recursion cycle here: `msort` calls `msplit`/`merge`
  and itself, while `msplit` and `merge` only self-recurse. This wording is misleading given the earlier
  corpus section that uses 'mutual recursion' for actual cross-calling functions."

## Liaison verification (confirmed on trunk 36e107eae)

Case "a full MERGE SORT …". The defs: `msplit` calls only `msplit` (self-recurse); `merge` calls only
`merge` (self-recurse); `msort` calls `msplit`, `merge`, and `msort`. So there is NO mutual-recursion
CYCLE — `msplit`/`merge` never call back to `msort`; it's a caller→helper structure where each function
is independently self-recursive (a recursion TREE, which the SAME doc sentence also correctly says: "a
real recursion tree, not linear"). "Mutual recursion msort->msplit->merge" is the wrong term (mutual
recursion = functions that call EACH OTHER cyclically, e.g. even/odd) and is misleading against the
corpus's genuine mutual-recursion cases. Fix: reword — drop "mutual recursion", call it a
divide-and-conquer / caller-with-two-helpers recursion tree (`msort` recurses on both split halves; the
helpers are each self-recursive). Doc-only, pins correct.

Owner: **corpus-bugfix** (`spec/semantics/09-functions.sexp`; `1f526f4bf`). Reword the "mutual recursion"
mischaracterization.
