# OPEN QUESTION: a whole handle in the NEXT-STATE argument (2026-08-19)

pyre3 — (resume (* s 10) (handle E 40 ... (+ (E.tick) 2))) with a x1000
toll. UNIFORM x2 (wasm=rust) and the DISTINCT-EFFECT differential
(inner=F) computes IDENTICAL values (415210 / 404200) — so semantics
are consistent and routing-independent. But BOTH diverge from the
constant-42 control (pyre3-const42: 47210 / 46200), so the handle in
next-state position does NOT behave as its value 42.

Single-dispatch reduction (one draw, next-state unused): matches the
naive model exactly (1010 / 0) — the divergence needs the next-state
to actually FEED a later dispatch. Decomposition attempts (per-dispatch
re-evaluation, routing) don't obviously produce 415210. ASKED v-effects
for the evaluation rule of handle-expressions in next-state position
(evaluated once at dispatch? re-run per replay? deferred to the next
dispatch?). Held unpromoted pending the ruling; oracles recorded as
OBSERVED values only.

RULING (tick 1854, v-effects): 415210 is a SILENT MISCOMPILE, correct is
47210. A CLOSED PURE handle in next-state position must be referentially
transparent (== its value 42 == (* 6 7)); the compiler threads the
arithmetic form correctly but re-splices/re-evaluates the HANDLE form in
the next dispatch's state slot. Uniform x2 = shared-fold miscompile
(uniformity rules out backend divergence, NOT correctness; the distinct-
effect differential confirms routing-independence, NOT the value).
v-effects acting on it NOW (silent-miscompile = act-instantly, unlike the
pyg1 ICE). Oracles set to ruled-correct 47210/46200:
- pyre3.sexp: FAILS at 415210 (miscompile-witness, flips on the fix).
- pyre3-const42.sexp + pyre3-distinct.sexp: the pass controls (const &
  distinct-effect both correct at 47210; distinct STILL miscompiles too
  actually — re-check on fix). Held as todo-witness, no baseline row.
LESSON REINFORCED: uniform x2 + distinct-effect-identical does NOT prove
correctness — only a MODEL (or a referential-transparency argument like
the const control) does. The const-42 control is what pinned the bug.

- `pyre4.sexp` (tick 1855) — IF-WRAPPED handle in next-state GATES the
  miscompile: n=10 selects the handle branch -> 415210 (WRONG, correct
  47210); n=0 selects a pure-3 constant -> 3300 (CORRECT, threads fine).
  Confirms the bug is precisely the closed-pure-HANDLE in next-state
  position, not next-state complexity generally: a constant in the same
  slot on the same machine threads correctly. Sibling miscompile-witness
  for the same fix; the n=0 row is a genuine PASS row today.

FIX INBOUND (tick 1856, MR a1188dc74 queued): the silent miscompile is
converted to a clean DECLINE (reject-not-miscompile) — a nested-handle
next-state is declined rather than threaded raw as a reduce_handle seed.
The durable correct-FOLD (reduce the handle to its value 42 and thread
it) is a deferred follow-on. So ON LAND:
- pyre3.sexp + pyre3-distinct.sexp: flip from wrong-VALUE 415210 to a
  clean fold-boundary DECLINE (todo-witnesses, NO baseline row, NOT
  pinned at 415210 nor 47210).
- pyre4.sexp: the HANDLE branch (n=10) now declines, so the whole case
  declines (held todo) — the n=0 pure-3 pass is subsumed.
- pyre3-const42.sexp: still PASSES at 47210 (pure-arith next-state folds)
  — the only promotable pass here.
On-land action: rebuild, confirm the declines, promote ONLY const42 as a
pass-pin; keep pyre3/pyre3-distinct/pyre4 as decline/todo-witnesses until
the correct-fold follow-on lands (then they flip to 47210 pass).

- `pyre5.sexp` (tick 1857) — MATCH-over-handle in next-state: the even
  arm selects the closed pure handle (n=0 -> 404200 WRONG, correct
  46200), the odd arm a constant 9 (n=10 -> 10910 CORRECT). Completes
  the wrapper sweep (if=pyre4, match=pyre5, let=v-effects-verified): the
  reaches_nested_handle guard RECURSES the whole next-state subtree so
  every wrapper declines as a whole arm on land. Ruled-correct oracles
  10910 / 46200 — n=10 passes today, n=0 flips from miscompile to
  decline on land then to 46200 pass on the correct-fold follow-on.

LANDED (tick 1858): the decline fix landed as 348bd4805. Fresh-build
re-gate: pyre3 + pyre3-distinct + pyre4 + pyre5 now DECLINE (todo, no
miscompile) — the silent 415210 wrong-answer is GONE. const42 PASSES.
So the miscompile class is closed as a clean reject. Oracles left at the
ruled-correct values (47210 etc.) so the probes auto-flip to pass when
the correct-FOLD follow-on lands; today they read as fold-boundary
todo-witnesses. Only const42 is a corpus pass-pin.
