# Pattern linearity must be pinned across sub-patterns, not only in a flat pattern

*2026-07-08*

**What happened.** Probing pattern linearity, I confirmed the seed does not yet enforce it —
`(match (tuple 1 2) ((tuple x x) x) (_ 0))` binds `x` twice and silently takes the second (→2),
which the corpus already records as a `(needs linear-patterns)` todo (the flat case). But the
corpus pinned only the FLAT repeat; it had no case for a name repeated ACROSS sub-patterns of a
composed pattern — `(tuple x (tuple x y))`, `(Some (tuple x x))` — which core-semantics.md
#Patterns Compose calls out explicitly: "a name appearing in more than one sub-pattern is the same
CDZ0102 error as one appearing twice in a flat pattern." The seed accepts these too (silently
shadowing the outer `x`), and no gate case guarded the nested facet.

**Why it matters even though linearity is unrealized.** Linearity is gated `(needs
linear-patterns)`, so both the flat and nested cases SKIP today — neither is a current FAIL, and
the seed's acceptance is an honest not-yet-realized decline-equivalent. The value of pinning the
nested case now is anticipatory: when a generation realizes linearity, the natural first
implementation scans a single pattern node's immediate binders for duplicates — which catches the
flat `(tuple x x)` but MISSES a name repeated in a nested sub-pattern (`(tuple x (tuple x y))`),
silently shadowing the outer binder exactly as before. That is the recurring "a check proven on
one form is not carried to its sibling" shape, here pre-empted at the corpus level: with the nested
case in place, a linearity fix that handles only flat patterns FAILs the gate, forcing the
recursive check the spec (#Patterns Compose) requires.

**The lesson.** When a rule is compositional by spec ("linear across the WHOLE pattern"), the
corpus should pin both the flat instance and the cross-sub-pattern instance, so a realizing
generation cannot pass by implementing only the shallow check. A `(needs …)`-gated capability is
the right time to add the nested companion — it costs nothing now (it skips) and closes the
"shallow check looks done" trap when the capability lands, exactly as the tuple-pattern-arity and
annotation-payload checks needed their nested companions to force recursion.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a pattern that binds the same name
across nested sub-patterns is rejected" — `(match (tuple 1 (tuple 2 3)) ((tuple x (tuple x y)) x) (_
0))` MUST reject CDZ0102 (gated `(needs linear-patterns)`, so it skips until the capability lands),
the nested companion of the existing flat `(tuple x x)` case. Native seed. Not a current FAIL — a
coverage-completeness addition that makes the eventual linearity fix prove the recursive check.
