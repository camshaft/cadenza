# Leading-rest list-binding unsoundness → CDZ0210 reject (operator ruling a) — SHIPPED, pending content-confirm

**Owner:** v-patterns. **Status:** MR 2d537d6ed sent (NOT yet on trunk as of 41880b4ae). Resolves the long-standing OPEN item `leading-rest-list-binding-unsound-vs-spec` (MEMORY root).

## What
Per the concierge's ruling (a): a leading-ELEMENT list-rest pattern (binds a leading element + a rest, e.g. `(a .. rest)`) is REFUTABLE — cannot match `[]` — so an irrefutable let/param BINDING of it is unsound; the compiler now REJECTS it CDZ0210 (same as a non-exhaustive match). The ZERO-leading rest (`(.. rest)`, whole list) is irrefutable and survives.

## Corpus revision (02-binding-and-control.sexp)
- 2 old value cases ("a def parameter may be a list rest pattern binding the head" + "a let binder may be a list rest pattern binding a leading element and the rest") → expect-(error CDZ0210) rejects.
- 1 new positive case ("a zero-leading list rest pattern binds the whole list in a def parameter") → binds whole runtime list, sum=60.
Baselines net +1 each backend; gates green all 3.

## corpus-bugfix confirmation
REASONING confirmed spec-correct (2026-07-19): leading-element rest refutable → reject; zero-leading irrefutable → binds. CONTENT-CONFIRM PENDING: 2d537d6ed not on trunk yet — will verify the 3 revised cases (rejects fire + positive binds sum=60) once it lands.
