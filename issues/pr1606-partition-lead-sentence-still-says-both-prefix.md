# PR #1606 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1606 (PR: "design: PR#1599 — clarify effect/* is
shorthand"). This IS the fix for my #1599 finding — they added the shorthand note. Copilot caught a
residual: the lead sentence of the same paragraph still frames BOTH as prefixes.

## Lead sentence still calls the split a "family-string prefix" for both, contradicting the new note (Copilot, :373) — doc/accuracy
> This paragraph still states the `control/*` vs `effect/*` split is a "family-string prefix", but the
> new note clarifies only `control/` is a literal prefix and effect families are bare. Reword the lead
> sentence so it accurately describes that the partition is based solely on the `control/` prefix, with
> everything else treated as a world-effect family.

VERIFIED on the cand branch: the lead sentence (:368) reads "The `control/*` vs `effect/*` split is a
real **family-string prefix**, tested by `family.starts_with("control/")`…" — then the NEW note (:371,
the #1599 fix) immediately clarifies "`effect/*` … is shorthand … those families are BARE … Only
`control/*` is a literal prefix." So the lead still frames both as prefixes while the note it precedes
says only `control/` is one. Small internal tension in the just-added text. FIX: reword the lead to
"The partition is the `control/` family-string prefix — a family is control-plane iff it
`starts_with("control/")`; everything else is a world-effect family (bare)." LOW/doc — closes the loop
on the #1599 clarification cleanly.
