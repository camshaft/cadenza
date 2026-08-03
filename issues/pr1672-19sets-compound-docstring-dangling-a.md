# PR #1672 review comment — spec/semantics/19-sets.sexp (v-runtime) — OPEN

https://github.com/camshaft/cadenza/pull/1672 (pin the COMPOUND-key CHAMP collision node — the #1650
CHAMP lineage). Author is v-runtime (cand/v-runtime-dac9dfa41), verified via gh — NOT corpus-bugfix
(19-sets is the shared-identity-prone zone; per the author-verify trap).

## Docstring references an unbound `a` — leftover from the prior Map case (Copilot, 19-sets.sexp:3160) — doc/accuracy
> The docstring references `a` in `(tuple (+ a 1) z)`, but this case does not introduce an `a` binding
> (the code uses the literal `150512887`). Looks like a leftover from the prior Map case.

VERIFIED on the cand branch: the docstring's absent-decoy phrase is "a tuple with a DIFFERENT first
element (`(tuple (+ a 1) z)`, whose hash almost surely does not collide) is absent" — but the case binds
no `a`; the input uses `(+ 150512886 z)` / `(+ 59555794 z)` literals. `a`/`b` were the prior Map case's
key names. Reword the decoy to the actual literal (`(tuple (+ 150512887 z) z)` or whatever the code
probes) so the rationale matches the case. LOW/doc — fold into the next 19-sets touch per the
no-standalone-polish steer.
