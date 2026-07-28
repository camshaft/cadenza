# PR#748 review comment — value-facts design doc: soundness direction is REVERSED

Mirrored from GitHub PR review comment (Copilot), id `3624148824`.
PR: https://github.com/camshaft/cadenza/pull/748 (merged; fix still belongs on trunk)
Location: `implementation/design/DESIGN-flow-sensitive-value-facts.md:100` (the `ValueFact` doc-comment)

## Comment (verbatim)

> The soundness description here is reversed: value facts used to justify check elision must
> conservatively over-approximate the values possible in the current flow context. A too-wide fact
> is safe but misses optimizations; a too-narrow fact is unsound (it could justify eliding a needed
> check).

## Liaison verification (CONFIRMED — Copilot is correct)

The doc-comment (lines ~101-103) reads:
> "A fact only ever NARROWS the true value set — a wrong-because-too-wide fact is unsound, a
> too-narrow one is just missed optimization."

This is BACKWARDS, and it contradicts the doc's OWN surrounding text:
- Line 96: "A `ValueFact` is a conservative **OVER-approximation** of the set of values a `StructId`
  occurrence may take" (i.e. `actual ⊆ fact`).
- Lines 103-104: "the join (at control-flow merges) WIDENS (set-union), the meet (at a refinement)
  NARROWS (intersection)" — the correct may-analysis lattice.

Derivation (check elision, e.g. div-by-zero): you know `actual ⊆ fact` and elide the check iff you can
prove `0 ∉ fact`.
- **Too-WIDE** fact (still ⊇ actual): may still contain 0 → can't prove elision → SAFE, just a missed
  optimization.
- **Too-NARROW** fact (⊊ actual, drops a real value like 0): you'd wrongly prove `0 ∉ actual` and elide
  a NEEDED check → UNSOUND.

So the correct statement is the inverse of what's written: too-wide = safe/missed-opt, too-narrow =
unsound. (The phrase "A fact only ever NARROWS the true value set" is also mis-stated — a fact
over-approximates, i.e. is a SUPERSET of, the true value set.)

Doc-only, but a SOUNDNESS statement in a design proposal that an implementer could follow into an
unsound check-elision — worth fixing before the design is built on. Owner: design agent (authored
`91b834723` "value-facts proposal — scope DECIDED"). Routed as a note.
