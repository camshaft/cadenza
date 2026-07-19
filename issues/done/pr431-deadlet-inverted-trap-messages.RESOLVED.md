# PR review comments — mirrored from GitHub PR #431 (Copilot inline) — deadlet.cdz inverted trap messages

- **PR:** #431 "fleet: fifty-fifth batch (compiler-ml deadlet, …)" (MERGED)
- **File:** `implementation/compiler-ml/src/deadlet.cdz` (lines 67, 73, 79, 91, 98, 122)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592257739, 3592257757, 3592257772, 3592257789, 3592257805, 3592257824
- **Links:** https://github.com/camshaft/cadenza/pull/431#discussion_r3592257739 (+ r3592257757, r3592257772, r3592257789, r3592257805, r3592257824)

## Comments (verbatim, condensed — all the same class)
Each `@test` in deadlet.cdz uses an `if <cond> then trap("<msg>") else …` (or the else-branch traps),
but the trap MESSAGE describes the SUCCESS condition, not the failure that actually triggered the trap:
- [67] `dl-drops-dead`: traps when the result is STILL a Let (dead let NOT dropped) but says "dead let dropped".
- [73] `dl-keeps-used`: traps when the used let WAS dropped but says "used let kept".
- [79] `dl-preserves-value`: traps when meaning is NOT preserved but says "meaning preserved".
- [91] traps when Let nodes REMAIN but says (as if) all dropped.
- [98] traps when the outer (shadowed) binding was NOT eliminated but says it was.
- [122] traps when `dle` is NOT idempotent but says it is.

## Liaison triage — CONFIRMED against trunk
Confirmed (spot-check line 67): `if is-let(dle(e)) then trap("dead let dropped") else (…)` — the trap
fires precisely when `dle(e)` IS still a Let (the dead let was NOT dropped), yet the message asserts the
success case. All 6 are inverted failure messages: the tests still trap on the correct CONDITION (so
they catch regressions), but a failure prints a misleading message that describes the opposite of what
went wrong. Low severity (test-message accuracy), but a coherent cluster worth one fix. compiler-ml
territory (v-compiler-ml owns deadlet.cdz). FIX: reword each trap to state the FAILURE (e.g.
"dead let was NOT dropped"). Fix on `trunk`. Quotes + links in queue file.
