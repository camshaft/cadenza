# BUG (ICE, closure conversion): tuple-index projection of a CAPTURED tuple inside a closure body → "parameter reference has no local slot"

**Status:** OPEN — the chr1 ICE's second, EFFECTS-FREE face. chr1 (a capture-once closure in a
recursive HOF, 14-effects) is parked with v-effects; this repro has NO effects and NO recursion —
a plain host-crossing closure — so the bug is in GENERAL closure conversion (the opt.rs slot
resolution the ICE-signature work already fingered), not the effects lane. Routed to v-effects as
the chr1 owner with the reframe; re-route to the closure-conversion owner as appropriate.

**Found by:** breaker tick 361 (the host-closure × immortal-era campaign's first probe).

## Minimal repro (tracked known-FAIL as hcx1, 21-host-closures)

```
(do (def (f (: n Int64)) (let ((a (tuple n 7))) (fn ((: q Int64)) (+ q (. a 0))))) (export f))
```
`(call f 1 5)` → ICE "parameter reference has no local slot". Boundary (all verified):

| closure body over the captured tuple `a` | result |
|---|---|
| `(. a 0)` (any tuple-index projection; flat or nested; one or two captures) | **ICE** |
| return `a` WHOLE (no projection) | works (value + census correct) |
| `(= a a)` eq-only | honest decline ("borrowing op operand… cannot yet prove") |
| captured LIST + List.at/len | works |
| captured immortal 33-trie + List.at | works |

The trigger is specifically the TUPLE-INDEX (`.`) lowering of a captured binding during closure
conversion — the same operator whose dup-release is dqe leg-1 (issues/BUG-nested-compound-dual-use…,
v-core-opt). Two distinct bugs on one lowering path; fixes are independent but adjacent.
