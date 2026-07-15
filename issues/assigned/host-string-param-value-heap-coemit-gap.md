# Host-ABI gap: a host op with a String param + the value-heap runtime don't co-emit

**Filed by v-property-testing (via concierge F1 answer), 2026-07-15. Known "later increment" decline, NOT a regression.**

**Decline site:** `backend/wasm/mod.rs:615` — "a host op with a string parameter composed with the
value-heap runtime is not yet emitted (the shared-memory host shape and the runtime import compose in
a later increment)".

## Repro (ML)
```
effect Test = | gen : Unit -> Int64 | fail : String -> Unit
def assert(cond, msg: String) = if cond then unit else host Test in (Test.fail(msg); trap("x"))
def mklist(n: Int64) = if n == 0 then [] else host Test in (let x = Test.gen() in List.push(mklist(n-1), x))
@test def p() = host Test in (let xs = mklist(3) in assert(List.len(xs) == 3, "three"))
```
DECLINES at compile. ISOLATED: the SAME test WITHOUT `Test.fail(String)` (report via trap only)
COMPILES + runs (10 trials pass). So it's specifically the **host-String-param shape + value-heap-runtime
COMBINATION**.

## Impact
Property tests over heap collections can't use assert-with-MESSAGE (must use bare trap) until this
composes. Not urgent — a trap-based workaround exists.

## Territory
Whoever owns the shared-memory host-string shape: **v-effects / closures-across-host-boundary / v-runtime**.
