# mlrepro: cross-type payload ctor-pattern silently matches a different sum type (SOUNDNESS)

Confirmed on trunk `38497484c` (+ my queued 580ecea2e/8a29963ac/af5bfe254, none of which touch this).
Owner: v-compiler-ml (this is the ii-c2b-2 "cross-type CDZ0203 reject" gap — co-designed with v-inference).

## Minimal repro

```
(do (type A (MkA Int64)) (type B (MkB Int64))
    (def (main) (match (MkA 5) ((MkB x) x) (_ 0)))
    (export main))
```

Expected: DECLINE (None) — a `B`-ctor pattern must not match an `A`-typed scrutinee (cross-type reject).
Actual: RUNS and returns 5 — the `((MkB x) x)` arm matched an `(MkA 5)` value and bound x=5. UNSOUND.

Control (same-type, must still run — and does): `(match (MkA 5) ((MkA x) x) (_ 0))` → 5. PASS.

## Root cause

- Each sum type restarts ordinal tags at 0 (per `read-do-ctors`), so `MkA` = tag 0 AND `MkB` = tag 0.
- `NMatchCtor(scrutId, ctorTag, binderId, bodyId, restId)` carries only the TAG, not the pattern ctor's
  decl. lower → `CMatchSum(scrut, tag, …)`; eval compares ONLY the stored tag against the scrutinee's
  stored tag. Both are 0 → the arm fires → a B-pattern reads an A-value's payload.
- The `NMatchCtor` infer arm (infer-db.cdz ~117) infers the scrutinee (types `TSum(A)` since ii-c2b-1)
  but NEVER checks the scrutinee's TSum decl against the pattern ctor's decl — so nothing rejects it.

## Fix plan (ii-c2b-2 slice, coupled — atomic, needs same-tick v-inference review)

1. Make the pattern ctor's DECL available at the NMatchCtor node. Cleanest: the reader already has the
   ctor NAME `cnm` (it calls `ctor-tag-of(name-id(cnm))`) — store the ctor NAME-id in NMatchCtor (either
   replace field-2 tag with name-id + have lower re-derive the tag via `ctor-tag-of`, or widen the node
   to carry both). Touches parse-db (node def/comment), sread (reader), lower-db (tag re-derive), infer-db.
2. infer NMatchCtor arm: look up the pattern ctor's decl (`ctor-decl-of(name-id)`) and the scrutinee's
   type; if the scrutinee is `TSum(scrutDecl)` and `scrutDecl != patternDecl` → reject (TErr, the
   CDZ0203 cross-type decline). Unify via the landed decl-identity `unify-ty` TySum arm.
3. Keep the arm-join (already landed 580ecea2e) + the single-field binder<-Int64 placeholder (ct
   argTypes[i] is the follow-on). Same-type patterns must still run (control above).

## Verified boundaries this doesn't regress (already-landed guards)

- multi-field payload → clean decline (ss-multifield-payload-ctor-*).
- nested single-field payload round-trips (ss-nested-payload-some-of-some-roundtrips).
- arm-type mismatch declines (ss-payload-pattern-arm-type-mismatch-declines).
