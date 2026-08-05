# PR #2297 review — rcdzc/src/tests.rs + spec/semantics/14-effects-and-handlers.sexp (v-effects) — OPEN — 1 convention + 1 comment-correctness + 1 whitespace [ALL VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2297 (pin the host-delegated next-state-slot perform as SERVED —
the as-class routing boundary, sibling of the as6 served-control from #2295). Copilot 3 inline.

## c1 — bare `HostResponse { op: "ask" }` vs documented dotted `ask.ask` (Copilot, tests.rs:68294, id 3724000044) — convention [VERIFIED, LOW] — SIBLING of #2295
Identical to the #2295 finding (id 3723968725): the pin uses bare `"ask"` where `cdz_run::HostResponse.op`
(cdz-run/lib.rs:311) documents a dotted `E.op` name (`ask.ask`). v-effects ALREADY decided this class is a
fix-forward-not-amend (won't orphan the queued correctness ref) — so this sibling occurrence rides the SAME
disposition: tighten `ask`→`ask.ask` when #2297 lands. No separate decision needed.

## c2 — the execution-trace COMMENT is internally inconsistent: "B.step returns 5 twice" vs the next line's "second B.step=105" (Copilot, tests.rs:68299, id 3724000104) — comment-correctness [VERIFIED, LOW]
> it says `B.step returns 5 twice`, but the next line uses `second B.step=105`. With state threading, the
> second `B.step` should read and return the advanced state (105).
VERIFIED in the diff. Line 91: "B.step returns 5 twice"; line 93: "first B.step=5 (state 5→105), second
B.step=105 → 10*5 + 105 = 155." The two lines contradict — line 93's "second B.step=105" is the one
consistent with the state-threaded advance and the asserted 155; line 91's "returns 5 twice" is the stray
wrong phrase (the SECOND read returns the advanced 105, not 5). LOW/comment-correctness (the pin's asserted
value 155 is fine — only the explanatory trace's "5 twice" clause is wrong). Fix: reword line 91 to "first
B.step returns 5, second returns the advanced 105" so the trace matches line 93 + the 155 assert.

## c3 — 6-7 consecutive blank lines between corpus cases vs the single-blank convention (Copilot, 14-effects-and-handlers.sexp:1317, id 3724000136) — whitespace [VERIFIED, LOW] — SAME class as my #2239
> multiple consecutive blank lines between cases here; elsewhere in this corpus file cases are separated by
> a single blank line (e.g. around lines 1239–1241). Collapsing these avoids unnecessary churn.
VERIFIED: the diff inserts 6-7 blank lines (diff lines 40-46) between the new host-delegated `(case …)` and
the following `(case "a handler arm forwarding …")`. The corpus convention (and my prior #2239 catch, landed
as #2268) is a SINGLE blank line between cases. LOW/whitespace. Fix: collapse to one blank line — same
cleanup as #2239/#2268. (This is exactly the double/triple-blank-run class I flagged before; worth v-effects
running the same single-blank sweep on the new case.)

v-effects owns rcdzc effects + the effects corpus. PR OPEN → all foldable pre-merge. c1 rides the #2295
fix-forward disposition; c2 + c3 are trivial same-commit cleanups.
