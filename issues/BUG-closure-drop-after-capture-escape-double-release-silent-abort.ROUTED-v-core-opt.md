# BUG (runtime abort): dropping a closure handle whose body RETURNED its capture crashes silently — double-release at the resource dtor

**Status:** OPEN — routed to `v-core-opt` (the heap_operand_ownership / shell-reclaim lane: this is
the #4425 pattern at the closure-resource seam). Found by the breaker host-closure campaign (tick 362).

**Symptom:** `(call f …) (drop)` where f's closure body returns its CAPTURED heap value → the run
dies with NO output and NO wasm trap message (the harness surfaces the stderr provenance line as a
bogus "trap"). A silent hard abort — likely the debug detector's abort path on the double-free.

## Trigger matrix (verified)

| shape | result |
|---|---|
| body returns the CAPTURED tuple, then (drop) | **silent crash** |
| body returns the CAPTURED list, then (drop) | **silent crash** |
| same shapes WITHOUT (drop) | pass (leak 1, the held handle) |
| body returns a FRESH heap value (scalar capture), then (drop) | pass (0) |
| body READS the capture (List.at/len), then (drop) | pass (0 — cascade correct, incl. immortal no-op) |

**Mechanism read:** the value-form encode of the returned capture on `call` consumes/releases the
capture's ref; the handle drop's t-dtor then releases the env again → double-free → abort. The fix
is the #4425 move: classify the encode's take as Owned (dup at the escape) OR suppress the dtor's
env release for escaped captures.

**Meanwhile:** hcz1 (tuple face) + hcz2 (list face) pinned as tracked known-FAIL rows in
21-host-closures; hcd1/hcd2 pin the CORRECT drop-cascade (mortal + immortal captures) that must
survive the fix. Also flagging the harness face: a run that dies without a trap should surface as
an ARTIFACT/ICE-class failure, not the stderr provenance line as the "trap" text (B1's sibling).
