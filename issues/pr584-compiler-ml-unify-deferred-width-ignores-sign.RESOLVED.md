# pr584 — compiler-ml unify: deferred-width int unifies across SIGN (+ test literal encoding) (2 Copilot)

Mirrored from GitHub PR #584 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/584 (10-MR publish batch)
File: `implementation/compiler-ml/src/unify.cdz` — both VERIFIED against `git show trunk`.

## SUBSTANTIVE — id 3608237775 (unify.cdz:53) — deferred width grounds without sign check
> `IntW` unification currently treats a deferred width (`width == 0`) as compatible with any integer
> *without checking signedness*. That allows a signed and unsigned integer type to unify whenever
> either side is deferred, which contradicts the stated intent that sign+width both identify distinct
> integer types (and differs from the seed compiler where sign unifies independently of width).

VERIFIED: the `Ty.IntW(sa,wa)` ~ `Ty.IntW(sb,wb)` arm:
```
if (wa == 0) then Result.Ok(s)                          // deferred ~ anything int → ok  (sa IGNORED)
else if (wb == 0) then Result.Ok(s)                     // anything int ~ deferred → ok  (sb IGNORED)
else if ((sa == sb) and (wa == wb)) then Result.Ok(s)   // concrete ~ concrete checks BOTH
else Result.Err("cannot unify integers of different width or sign")
```
The deferred arms return Ok without comparing `sa`/`sb`, so a signed deferred unifies with an unsigned
concrete (and vice-versa). The concrete-concrete arm DOES check sign, so the deferred paths are the
gap. Copilot notes rcdzc (seed) lets sign unify independently of width — so the FIX direction (should
a deferred width also ground its sign, or must sign always agree?) is a compiler-ml inference-semantics
call, not obvious from the code alone. Real, worth v-inference's judgment.

## TEST-SIDE — id 3608237779 (unify.cdz:169) — deferred literal `IntW(0,0)` collides with unsigned encoding
> The deferred-width tests use `Ty.IntW(0, 0)` as the deferred literal, but `0` is also the encoding
> for *unsigned* in `IntW(signed 1|0, width)`. After enforcing signedness compatibility (and to keep
> the representation consistent), these deferred-width tests should use `Ty.IntW(1, 0)` for a signed
> deferred-width literal when unifying with `Int8`.

VERIFIED: `unify-intw-deferred-grounds-right` (unify.cdz:169) does `unify(empty(), Ty.IntW(1, 8),
Ty.IntW(0, 0))` — the deferred literal `IntW(0,0)` has sign-field 0 = unsigned, unifying with the
signed `Int8` `IntW(1,8)`. This test only passes TODAY because of the #53 gap (deferred skips sign). If
#53 is fixed to require sign agreement, this test's deferred literal must become `IntW(1,0)` (signed).
So #53 and #169 are ONE change: fixing the sign-check + updating the deferred test literals together.

## Owner
`compiler-ml/src/unify.cdz` = v-inference (owns infer/unify/resolve). Both together.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk dcc81d629): the deferred-sign gap is FIXED in
implementation/compiler-ml/src/unify.cdz. Sign is now a 3-state axis {1=signed, 0=unsigned, 2=DEFERRED}
(header comment 13-21); `sign-agrees` (line 44) grounds a DEFERRED sign (2) to the other + rejects a
fixed-signed/fixed-unsigned mismatch (CDZ0301), checked INDEPENDENT of width. A bare literal is now
`IntW(2,0)` (both-deferred), NOT the old `IntW(0,0)` (spuriously unsigned). The deferred test literals were
updated to match (`unify-intw-deferred-grounds-left/right/meets-deferred` all use `IntW(2,0)`, lines 174-186)
and there's a dedicated `unify-intw-sign-mismatch` test (line 169). Exactly the "#53 + #169 as ONE change"
the review asked for. Owner (v-inference/v-compiler-ml PORT) resolved — no corpus-bugfix action.
