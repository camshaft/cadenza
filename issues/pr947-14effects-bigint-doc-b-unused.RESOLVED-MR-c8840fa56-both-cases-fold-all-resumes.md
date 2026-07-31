# PR#947 review comment — 14-effects BIGINT case doc overstates ("each resume") but `b` is bound-unused (corpus-bugfix)

Mirrored from GitHub PR#947 review comment (Copilot), id `3691605632` (:6362, also :6368).
File: `spec/semantics/14-effects-and-handlers.sexp` — corpus doc/coverage → corpus-bugfix. Blame
`ee9171e9a` "corpus(effects): 3-pin drain AJ — … BigInt and Rational handler states".

## Comment (verbatim)

- (id 3691605632, 14-effects-and-handlers.sexp:6362) "The BIGINT case docstring claims that 'each resume
  returns the PRIOR product', but the program never observes the second resume result (`b` is bound and
  then unused). Either incorporate `b` into the asserted output, or adjust the docstring so it doesn't
  overstate what's being checked. This issue also appears on line 6368 of the same file."

## Liaison verification (confirmed on trunk 16f366838)

Case "a BIGINT handler state multiplies per perform and each resume reads the prior product". Body:
`(def a (Acc.grow k)) (def b (Acc.grow 10)) (def c (Acc.grow 10)) (Int64.of (/ c (* a (BigInt.of 10))))`.
The final read uses `a` (1st resume) and `c` (3rd resume) but NOT `b` (2nd resume) — `b` is bound and
unused. The doc "each resume returns the PRIOR product" implies all three resume results are checked, but
`b` is never observed (output 7 = `c/(a·10)`). So the doc overstates coverage. Fix (Copilot's, either):
(a) fold `b` into the asserted output (e.g. a digit encoding all three) so "each resume" is actually
pinned, OR (b) soften the doc to say only `a` and `c` are read. The `:6368` sibling (the Rational-state
case in the same drain) is flagged same-class — verify its doc vs which binders it observes. Doc/coverage,
pin correct as-is.

Owner: **corpus-bugfix** (`spec/semantics/14-effects-and-handlers.sexp`; `ee9171e9a`). Either pin `b` or
soften the "each resume" claim (+ check :6368).
