# PR #1861 review comment — spec/semantics/14-effects-and-handlers.sexp (breaker) — OPEN

https://github.com/camshaft/cadenza/pull/1861 (the #1854 Core::And-in-or-case doc fix).

## Reworded "not a typo in this or case" phrasing is itself awkward (Copilot, 14-effects.sexp:7478) — doc/style
> Doc string reads "not a typo in this or case", which is ambiguous/grammatically off in context.
The #1854 fix (clarifying Core::And is the shared and/or node) reworded to "not a typo in this or case" —
which reads awkwardly (parses as "or case" ambiguously). Reword to something like "Core::And is the shared
and/or core node — this `or` case correctly references it (not a typo)." LOW/style. Fold into the next
14-effects edit.
