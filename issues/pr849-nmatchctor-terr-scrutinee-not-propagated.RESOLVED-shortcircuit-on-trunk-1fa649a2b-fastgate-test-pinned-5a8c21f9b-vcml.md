# PR#849 review comment — NMatchCtor inference doesn't propagate a TErr scrutinee (contradicts "Any TErr propagates")

Mirrored from GitHub PR review comment (Copilot), id `3647225238`.
PR: https://github.com/camshaft/cadenza/pull/849 (merged; fix belongs on trunk)
Location: `implementation/compiler-ml/src/infer-db.cdz:134` (`NMatchCtor` arm), landed `90d4b263e`.

## Comment (verbatim)

> In `NMatchCtor` inference, a `TErr` scrutinee currently doesn't propagate to the match expression
> type (only cross-type mismatch or arm-join failures do). This contradicts the preceding comment
> ("Any TErr propagates") and can allow an ill-typed scrutinee subtree to be masked by well-typed
> arms.

## Liaison verification (CONFIRMED on trunk)

The `NMatchCtor` arm (infer-db.cdz ~128-138): `m1 = infer-node(scrutId)`, bind binder Int64, `m2 =
infer-node(bodyId)`, `m3 = infer-node(restId)`, then:
```
(if (ctor-pattern-cross-type(...)) then TErr
 else (match lookup(m3, bodyId) with
   | Some(bt) => (match lookup(m3, restId) with
       | Some(rt) => join-branch-types(bt, rt)
       | None => TErr)
   | None => TErr))
```
The match type is derived from the cross-type check + the body/rest arm join. The SCRUTINEE's type
(`lookup(m3, scrutId)`) is never consulted — so a `TErr` scrutinee (an ill-typed subtree being matched)
does NOT force the match to `TErr`; well-typed arms mask it. The preceding comment (~122) explicitly
says "Any TErr propagates", so code ≠ doc, and it's a soundness-adjacent gap (an ill-typed scrutinee
should taint the whole match).

Fix: after inferring the scrutinee, if `lookup(m3, scrutId)` is `TErr`, short-circuit the match to
`TErr` (before/alongside the cross-type + arm-join checks) — matching the integer `NMatch`/`if` TErr
discipline. Add a compiler-ml test: `(match <ill-typed-scrut> ((Some x) 1) …)` → TErr, not the arm type.

Owner: v-compiler-ml (`compiler-ml/*` port; NMatchCtor shape `90d4b263e`, "per v-inference"). Routed as
a note flagged CORRECTNESS (mask of an ill-typed scrutinee).
