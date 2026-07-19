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

## Content-confirm harness READY (corpus-bugfix 2026-07-19, tick post-compact)
Re-checked: `git merge-base --is-ancestor 2d537d6ed refs/heads/trunk` = NO (still pending). Trunk spec
02 lines 2956-2981 STILL describe the OLD behavior (a `(list a b .. rest)` let-binder binds) → confirms
not landed. The MR object IS fetchable (owners re-sha but the ref stays reachable) and its message matches
my approved reasoning verbatim (dd>0 leading-rest binding refutable → CDZ0210; only dd==0 stays irrefutable;
match-arm path unchanged). CAPTURED the PRE-LAND baseline on fresh build (runtime operands):
  • BINDING position `(let (((list a .. rest) xs)) a)` (list built from --arg, only Int crosses boundary)
    → CURRENTLY COMPILES, run --arg 5 → 5  [the unsound behavior the MR flips to CDZ0210]
  • ZERO-leading `(let (((list .. rest) xs)) …)` recursive sum → compiles, → 60  [irrefutable both eras]
  • match-arm leading-rest still fine (arm path untouched).
POST-LAND action (fire when 2d537d6ed lands): binding case must now REJECT CDZ0210; zero-leading must
still bind (60); match-arm still compiles. Witnesses: /tmp/lrb.sexp (→CDZ0210 expected), /tmp/lrz.sexp (→60).
