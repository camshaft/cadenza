# Effect-handler mutation-vs-query state optimization — feasibility

Owner: v-effects. Status: SCOPING (operator idea via concierge, 2026-07-18). An OPTIMIZATION, not
correctness — lower priority than the tier-1/tier-2 miscompiles + E5. This note answers the concierge's
three questions: (1) analyzable? (2) worth it? (3) composes with the out-state fix + step 3?

## The idea

If a handler's state is only QUERIED (read, never changed), don't thread the state OUT as a return value —
pass it only as an ARG (in). A `State.get` that resumes with the current state unchanged doesn't need the
out-state threaded back; that threading is wasted work + emit size.

## (1) Is it statically analyzable? — YES, cleanly.

The classification is a structural property of each arm's `resume`:

- **QUERY arm**: `(resume v s)` where the next-state `s` resolves EXACTLY to the arm's own `state` binder
  occurrence — the state passes through UNCHANGED. Detected by: `tail_resume_next_state_of(arm.body)`
  resolves (`resolved_of` → `Ref`/`Param` chain) to `arm.state`. The corpus `now`/`get` arms are exactly
  this: `(now (u) s (resume s s))`, `(get (u) s (resume s s))`.
- **MUTATION arm**: `(resume v s')` where `s'` is any other expression (`(+ s 1)`, `(Map.insert s k v)`, a
  literal, a match-valued next-state). The state changes.

A **QUERY-ONLY HANDLER** = every arm is a query arm (or has no resume — an abortive arm never threads state
forward anyway). For such a handler the state is loop-invariant: it is the seed, unchanged, at every perform.

Edge cases the analysis must handle (all decidable):
- A `do`-wrapped / `let`-wrapped / `match`-wrapped resume (the peels): unwrap to the tail resume(s) first
  (reuse `peel_resume_from_arm_body`, which already yields the next-state), then test each peeled next-state
  against `arm.state`. A `match`-valued next-state is a mutation unless EVERY branch's next-state is the bare
  binder (rare; conservatively treat a compound next-state as mutation).
- A recursive-specialized handler (`f#ctx(state)`): the state param is threaded through self-calls; query-
  only means the self-calls pass the same state param unchanged. Detectable but the specialization already
  has its own state-param machinery — see (3).
- CONSERVATIVE default: if unsure, classify as mutation (thread the out-state — the current behavior). The
  optimization only ever REMOVES threading for a provably-unchanged state, so a false "mutation" is just the
  status-quo (no miscompile risk).

## (2) Is the ABI win worth it?

The win is real but NARROW in the current fold. Today `reduce_handle` folds a tail-resumptive handler AWAY
at compile time (the state threads as ordinary `let`/operand values in the rewritten body; there is no
runtime "handler state ABI" for the frame-free cases). So for the FOLDED cases, a query-only handler already
emits no out-state return — the state is just substituted as a value. The optimization's payoff is therefore
concentrated in:
- The **recursive-specialization** path (`specialize_recursive` / `thread_returning_tuple`): a recursive
  effectful walk emits `f#ctx(args…, state) -> (value, out-state)` (the #13 multi-value-return work). A
  QUERY-ONLY handler over such a walk could emit `f#ctx(args…, state) -> value` (drop the out-state tuple
  component) — a smaller return ABI + no per-call out-state threading. THIS is where the operator's win
  lands: the multi-value tuple return shrinks to a scalar for a query-only recursive handler.
- Any future runtime handler-state rep (step 3's stored continuations, a general handler-state cell): a
  query-only handler needs only an in-arg, no out-cell.

So: worth it MAINLY as a specialization-return-ABI trim (drop the out-state tuple slot for a query-only
recursive handler). For the non-recursive frame-free folds it is already effectively free (no out-state ABI
exists). Estimated: a bounded change to `specialize_recursive` (detect query-only → emit single-return even
where `force_multivalue` would have tupled) + the classifier. Medium value, low risk (conservative default
= status quo).

## (3) Composition

- **With the #13 out-state fix / multi-value return**: composes cleanly — it is the INVERSE gate. Today
  `force_multivalue` UPGRADES a callee to `(value, out-state)` when a caller observes the out-state. A
  query-only handler means NO arm changes the state, so the out-state a caller observes is always the
  unchanged seed — `force_multivalue` need not fire (the caller can read the seed directly). So the opt is:
  "if the handler is query-only, suppress the multi-value upgrade even when a caller observes the state
  (the observed out-state = the in-state)." A one-predicate gate on the existing machinery.
- **With step 3 (paused/partial)**: a query-only handler cannot store a continuation that later MUTATES the
  state (there is no mutation), so it is orthogonal — step 3's escaping-k is about the continuation, not the
  state-change classification. No conflict; the query-only trim applies to the state ABI independent of
  whether k escapes.

## Recommendation / slotting

FEASIBLE + worth a bounded increment, but it is an OPTIMIZATION that slots AFTER the correctness work:
1. (in flight / done) the tier-1/tier-2 miscompile fixes + E5 steps 1–2 + the DES single-task gate.
2. TRUE step 3 (multi-task escaping-k heap rep) — when v-discrete-event-sim files the 2-task repro.
3. THEN this query-only trim: a `handler_is_query_only` classifier (per the (1) detection) + a gate in
   `specialize_recursive` to emit single-return for a query-only recursive handler + suppress the
   `force_multivalue` upgrade. Gated by a differential emit-size probe (a query-only recursive handler emits
   a scalar return, not a tuple) + the full corpus (no behavior change — same values, smaller emit).

No correctness stake; the conservative default (classify-as-mutation-when-unsure) means it can never
miscompile. Recommend building it after step 3, as a focused emit-size increment.
