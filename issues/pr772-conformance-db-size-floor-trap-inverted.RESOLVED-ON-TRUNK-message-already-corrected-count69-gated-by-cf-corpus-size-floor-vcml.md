# PR#772 review comment — conformance-db size-floor trap message is inverted

Mirrored from GitHub PR review comment (Copilot), id `3628370916`.
PR: https://github.com/camshaft/cadenza/pull/772 (batch-staging; fix belongs on trunk)
Location: `implementation/compiler-ml/src/conformance-db.cdz:808`

## Comment (verbatim)

> The failure message is misleading: this `trap("corpus has 66 cases")` is only executed when the
> corpus size is *not* 66, so the message reads as the opposite of what happened. Updating it makes
> size-floor failures easier to diagnose.

## Liaison verification (CONFIRMED on trunk/staging)

conformance-db.cdz:808:
```
if List.len(corpus()) == 66 then unit else trap("corpus has 66 cases")
```
The `trap` fires only in the ELSE branch — i.e. when `List.len(corpus()) != 66` — but its message
asserts "corpus has 66 cases", the opposite of the actual condition. A size-floor failure would print
a message claiming the invariant HELD.

Fix: message the actual failure, e.g. `trap("conformance corpus size changed — expected 66 cases")` (or
include the actual count if the trap surface allows interpolation). Doc/diagnostic-only, no behavior
change (the guard logic is correct; only the message misleads).

Owner: v-compiler-ml (`compiler-ml/*` source — the conformance corpus). Routed as a note. Minor.
