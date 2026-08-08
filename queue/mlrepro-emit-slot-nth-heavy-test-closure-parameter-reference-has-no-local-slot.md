# emit codegen: "parameter reference has no local slot" trips at the Nth heavy @test closure in one module

**Filed by:** v-compiler-ml (ARC-B3, 2026-08-08)
**Surfaced building:** `implementation/compiler-ml/src/unify-subst.cdz` (the HM Subst-threading unifier).

## Symptom
`cdz check <file>` passes (exit 0, no errors), but `cdz test <file>` fails at EMIT time with:

```
cdz: error: parameter reference has no local slot
```

No source location is reported. The whole test suite fails to run (nothing emits).

## What triggers it
NOT a logic error and NOT a simple test-count ceiling. It is an emit/slot-allocation limit that trips
once a module accumulates enough "heavy" `@test` closures (tests whose bodies nest several
`match`/`if` over calls that thread a record param — here `Subst`-threading `unify-*-su` calls).

Empirically, in unify-subst.cdz (defs = unify-sign-su/unify-width-su/unify-int-su/unify-ty-su/
unify-ty-list-su + bind-or-check-sign/width, all of which check + run fine individually):

- Any **5** of the heavy B3 tests in one file → PASS (verified with two different 5-test subsets).
- A **6th** heavy test → `parameter reference has no local slot`.
- BUT a 6th *trivial* test (`if 1 == 1 then unit else trap`) → PASS. So it is the CLOSURE COMPLEXITY
  budget of the emit unit, not the count.
- ORDER-SENSITIVE: a 4-test companion file {concrete-conflict, occurs, bool, fn} FAILED, while a
  5-test file {concrete-conflict, occurs, bool, fn, **int**} PASSED — adding a test made it pass.
  So the slot-allocator's behavior depends on emit order / total closure shape, not a clean count.

## Minimal-ish repro
Build a module with the B3 `unify-sign-su`/`bind-or-check-sign` defs (see unify-subst.cdz) plus
~6 `@test`s each of the form:
```
@test
def t() = match unify-sign-su(subst-empty(), Sign.SVar(0), Sign.Signed) with
  | Option.Some(su) => (match apply-sign(su, Sign.SVar(0)) with | Sign.Signed => unit | _ => trap("x"))
  | Option.None(_) => trap("y")
```
`cdz check` clean; `cdz test` → slot error once the closure budget is exceeded.

## Impact / workaround
Blocks putting a full test suite for a Subst-threading module in one file. WORKAROUND used to ship
B3: keep 5 tests in unify-subst.cdz (green), defer the remaining pins (they are largely redundant
with hm-vars.cdz occurs tests + unify-ty.cdz concrete-conflict tests). This is a real emit limit to
fix, not just a file-split nuisance — a hand-written program with enough heavy closures in one
module would hit it too.

## Likely area
rcdzc backend local-slot allocation for closures/`@test` thunks — a param reference (`su`, the
threaded Subst) resolves to no local slot when the emit unit's slot pressure crosses some bound.
Routed to the compiler backend owner (v-compiler-perf / rcdzc backend) — NOT a compiler-ml logic bug.

---

## SHARPER ROOT CAUSE (ARC-B5 update, 2026-08-08) — it's CROSS-MODULE CALLS to a Subst-threading fn, not test count

Building ARC-B5 (hm-recsolve.cdz, a connected constraint solver over unify-subst) I isolated the trigger far more precisely — the "Nth heavy closure" framing above was a symptom, not the cause:

**A NEW module that imports `unify-ty-su` (from unify-subst.cdz) and calls it EVEN ONCE, non-recursively, fails `cdz test` with "parameter reference has no local slot" — while `check` passes and unify-subst.cdz's OWN tests (same function, same-module callers) pass.**

Minimal repro (fails):
```
import { Ty } from "ty"
import { Subst, subst-empty } from "subst"
import { unify-ty-su } from "unify-subst"
def solve1(su: Subst, a: Ty, b: Ty) = unify-ty-su(su, a, b)
import { ty-int64 } from "ty"
@test
def rs-one() = match solve1(subst-empty(), ty-int64(), ty-int64()) with
  | Option.Some(_) => unit | Option.None(_) => trap("e")
export { solve1 }
```
- Same-module call to `unify-ty-su` (its own tests) → EMITS FINE.
- Cross-module call (the above) → "parameter reference has no local slot".
- Not the `Constraint` sum wrapper (fails with a plain `Tuple(Ty,Ty)` too), not recursion (fails non-recursive), not test count (fails at ONE test / ONE call).

**Hypothesis:** local-slot allocation for a CROSS-MODULE call to a function that both takes AND returns a record/sum threading a param (`Subst -> ... -> Option(Subst)`) — the callee gets inlined/specialized across the module boundary and a param reference resolves to no local slot. Likely in rcdzc backend's cross-module inline/specialize + slot assignment.

## IMPACT — this BLOCKS the HM live-wire (operator's headline)
ARC-B1..B4 (subst/hm-vars/unify-subst/hm-scheme) all pass because their tests call their functions
IN THE SAME MODULE. But the whole point of the HM arc is to have OTHER modules (hm-recsolve, and
ultimately the infer pipeline) CALL `unify-ty-su`/`instantiate`/`generalize` — and that cross-module
call is exactly what breaks emit. So B5 and the infer→unify-subst live-wire are BLOCKED until this is
fixed. Requesting priority from the backend owner (v-compiler-perf / rcdzc backend): this is not a
test-ergonomics nuisance, it gates the operator's "scale to the next feature set with HM" deliverable.
