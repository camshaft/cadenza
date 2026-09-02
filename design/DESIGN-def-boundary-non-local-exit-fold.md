# DESIGN — def-boundary non-local-exit fold (multi-tick, v-effects)

Status: **DESIGN — cleared for a multi-tick effort (operator ruling via concierge, 2026-09-03).**
Process directive: write this design, bank it, then **reboot with a fresh context** to implement the
whole family to done (not just the one #7766 case). This doc is the durable hand-off for that reboot.

## 0. Goal & scope

Drive the **def-boundary non-local-exit** effect-fold family to done: an abortive perform (an effect op
whose handler arm does NOT `resume` — e.g. `(bail (n) s n)`) that sits at a position the tail-resumptive
fold cannot lift must ABANDON the pending computation and home to its handler boundary, folding to the
abort value (never a wrong value, never a leak). Each case currently DECLINES cleanly (CDZ0900 safe floor)
— the corpus pins the idealistic value as a tracked-TODO. This arc flips them to PASS via coded folds.

**Already landed** (the tagged-return calling convention + its extensions):
- #7613 — non-tail self-call abort (`(+ 1 (walk …))` → 99). `thread_returning_tagged` v1.
- #7640 — pending-in-handle-body (`db.force_tagged_abort`, adv-52): a tail-recursive abortive callee whose
  abort must abandon a pending op at the OUTER call site.
- #7642 — mutual-SCC threading (`mutual_scc_of`; a partner call is a tag-check-short-circuit).
- #7766 — **def-boundary match-arm foreign-arg** (14:12783): `(let ((a (unwrap (E.fetch) tag))) K)` where
  `unwrap` aborts in a MATCH arm + a FOREIGN-perform arg. Fold = `hoist_match_abort_let` (reduce.rs):
  inline the helper + distribute K into the NON-abortive arms only.
- #7790 — **accum-off-tail** (14b:13175): accumulator-introduction rewrote a non-tail associative abortive
  recursion into a TAIL self-call whose ACCUMULATOR ARG carries the abort. Fold = gate relaxation
  (`accum_off_tail`) + `thread_returning_tagged` direct-tail-self-call arm distributes/collapses the
  aborting arg (`abortive_arg_tuple`) + a `hoist_once` deferred-numeric type-agreement fix.

**Remaining family (this arc's targets)** — wasm-baseline `todo`, all in `14`/`14b`:
1. **INDIRECT accum abort** — "an INDIRECT (helper-hidden) abortive perform in a non-tail accumulator
   recursion" (14b, the interproc sibling of 13175/#7790): `(+ (loop (- k 1)) (helper k))`,
   `helper k = (if (= k 2) (E.bail 99) k)`. accum reassociates `(helper k)` onto the accumulator arg where
   the abort rides buried in a callee. CLOSEST follow-on to #7790.
2. **and-short-circuit self-call** — "a self-call gated behind an `and` short-circuit in an if-condition
   declines cleanly, never hoisted". The `and`/`or` connective desugar in `hoist_once` (reduce.rs ~2517)
   already turns `(and lhs rhs)` with an abort in `rhs` into `(if lhs rhs false)`; wire it to the tagged CC.
3. **conditionally-resuming (abortive-or-resume) arm reading the enclosing fn's param** — a mixed arm that
   both resumes AND aborts, reading an enclosing-fn param. Interacts with the resumptive fold's capture.
4. **abx3 mixed resume+abortive arms over a growing LIST state whose ABORT arm reads the STATE** — the abort
   arm reads the handler state (the tagged perform-leaf arm currently requires `count_param_refs(arm.body,
   arm.state)==0`, thread.rs:282 — a state-reading abort declines). Needs the abort value to thread the state.
5. **non-tail inner handle with a foreign perform sibling** ("needs frames") — a nested inner handle whose
   sibling performs an OUTER effect; the fold cannot represent the pending frame lexically.
6. **continuation call whose body ITSELF performs** — a reified continuation `(k …)` whose body re-performs
   the handled effect; needs handler-re-entry-at-apply (inc-2b). Likely EXCLUDE (borders multi-shot).
7. **glb1 collapse** — continuation-duplication join-point lowering (see
   `design-glb1-continuation-duplication-join-point-lowering`). A join point so a multi-use continuation is
   not duplicated. Its own sub-design exists; fold it in or keep separate per complexity.

EXCLUDED (per operator standing directive — multi-shot/multi-resume): mrs1 two-resume, frb3 non-tail-resume,
Ty::Cont escaping-k, pqueue-filed-k, adv-69 a3/a3-direct (nested-arm resume-value = multi-shot).

## 1. The convention (what a fold must preserve)

An abortive perform's specialized callee returns a TAGGED tuple `#tuple(tag value)`: tag 1 = abort, tag 0 =
normal. Every pending frame (a strict op / self-call / continuation wrapped around a sub-expression that may
abort) SHORT-CIRCUITS on the abort tag — it propagates `#tuple(1 v)` up UNCHANGED, abandoning its pending
work; on tag 0 it applies its pending op to `(. r 1)` (the normal value). The handle collapses the tagged
tuple to the value (`(. r 1)`) at the boundary. Helpers: `build_tag_tuple` (thread.rs), `build_spec_call`,
`tuple_proj`, `tagged_int_lit`.

**Two dual mechanics** (both landed, reuse them):
- **DISTRIBUTE a continuation/op into the non-abortive arms** (the #7766 match-arm shape + the strict-op
  `hoist_conditional_abort`): `(op … (if c (abort) e) …)` ≡ `(if c (op … (abort) …) (op … e …))`, then the
  aborting branch COLLAPSES (the op is abandoned) while the non-abortive branch keeps the op. Sound because
  an abort abandons the enclosing computation (pushing the op into the aborting branch, where it never
  completes, changes nothing) AND the op's other operands are pure (duplicating them across branches is
  observably identical).
- **COLLAPSE a strict frame whose operand aborts unconditionally** → the abort tuple (`abortive_arg_tuple`,
  #7790): the abort fires in strict eval order before the frame, so the frame is abandoned.

## 2. Key code sites (banked map — do NOT re-explore)

- **Gate** (route-vs-decline): `effects/mod.rs` `specialize_recursive`, the `tagged_abort` computation at
  ~4508 and the safe-floor declines at ~4544 (`!tagged_abort && abortive_perform_off_tail` → `return None`)
  and ~4561 (mutual). `abortive_perform_off_tail` (mod.rs:4395) detects an abort at a non-tail position
  (operand / call-arg / if-cond / let-init). `accum_off_tail = all_tail && off_tail && !ctx.abortive.is_empty()`
  (#7790) is the accum trigger. Add new triggers here for new shapes, NARROWLY (keep resumptive/other shapes
  untouched — the gate's `!ctx.abortive.is_empty()` guard already excludes resumptive accum).
- **Threader**: `effects/thread.rs` `thread_returning_tagged` (~231). Arms: `if` (pure cond → thread
  branches), `match` (pure scrut → thread arm bodies), perform-leaf (abortive → `#tuple(1 abortval)`, guarded
  `count_param_refs(arm.body, arm.state)==0` → a STATE-READING abort declines = case #4 above), direct-tail-
  self-call (~303; now distributes a conditional-abort arg + collapses an unconditional-abort arg via
  `abortive_arg_tuple`), strict-operand-self-call (~311; `(if (= (. r 0) 1) r <op-on-(. r 1)>)`).
- **Hoist**: `reduce.rs` `hoist_conditional_abort` (2396) + `hoist_once` (2412). 🪤 **`hoist_once` is
  TOP-ONLY** — it does NOT recurse into `if`-branches; apply it to the exact node whose direct operand is the
  if-abort (e.g. the self-call ARG), not an outer `(if … )`. Handles: strict-op-with-if-abort-operand (2415),
  strict-op-with-let-abort-operand (2483), `and`/`or`-with-abort-rhs desugar to `if` (2517). types_agree
  (2444) now treats a deferred-width numeric as undetermined (#7790) — reuse.
- **#7766 helper**: `reduce.rs` `hoist_match_abort_let` + the guard `init_is_foreign_arg_match_abort_call`
  (reduce.rs:2167) + `callee_aborts_in_match_arm` (2202).
- **Entry**: `thread_returning_tagged` is first called at `mod.rs` ~5016 (in `specialize_recursive` when
  `tagged_abort`). Recursive calls in thread.rs (branches/arms).
- **abort value**: `abortive_perform_value_ty` (reduce.rs:2285) = type_of the abortive arm body.

## 3. Per-case fold approach (design)

- **(1) INDIRECT accum abort**: like #7790 but the abort is inside a HELPER `(helper k)` the per-step term
  calls. accum's `compute_performing_defs`/`term_calls_performing_def` currently DECLINES accum when the
  per-step term calls a performing def (so the recursion stays a plain non-tail form the safe floor rejects).
  Approach: allow accum to proceed for this shape, INLINE the helper into the accum arg (so the abort is
  visible), then the #7790 accum-off-tail machinery folds it. Risk: the helper inline must preserve the abort
  semantics; A/B main(1)=1/main(3)=99 + --guarded-all.
- **(2) and-short-circuit**: `hoist_once`'s and/or→if desugar (2517) already lifts `(and lhs (abort))` to
  `(if lhs (abort) false)`. Confirm it fires for the self-call-in-if-condition shape + route to the tagged CC
  via the gate (a new narrow trigger, or the existing off-tail path once the abort is in a branch).
- **(3) conditionally-resuming arm**: an arm that both resumes and aborts. The tagged CC's perform-leaf arm
  handles a pure abort; a mixed resume+abort arm needs the resume path (tail-resumptive fold) AND the abort
  path (tagged) co-existing. Likely the hardest; may need the multi-value + tagged modes to compose. Defer
  within the arc if it blocks; it may be a separate increment.
- **(4) abx3 state-reading abort**: relax the perform-leaf arm's `count_param_refs(arm.body, arm.state)==0`
  guard — thread the handler STATE into the abort value (the abort arm reads `s`). The abort tuple becomes
  `#tuple(1 <arm-body-with-state-threaded>)`. Needs the state value available at the abort site (the tagged
  threader carries `states`); substitute the state binder → the threaded state expr. Heap state → --guarded-all.
- **(5) foreign-perform-sibling non-tail inner handle** ("needs frames"): a nested inner handle whose sibling
  performs an outer effect at a non-tail position. The lexical fold cannot represent the pending frame across
  the inner handle. This is the deepest; may need a genuine frame representation (or stay declined + document
  as out-of-scope for the lexical CC). Assess feasibility during implementation; do NOT force a miscompile.
- **(6) continuation-body-performs**: EXCLUDE (borders multi-shot / handler-re-entry-at-apply).
- **(7) glb1 collapse**: see the existing glb1 join-point sub-design; a multi-use continuation join point so
  the continuation is not duplicated. Fold in only if it shares the tagged-CC machinery; else keep separate.

## 4. Correctness argument (the guardrail — this is miscompile-risky)

The core soundness claim: **an abort abandons the pending computation**. So (a) distributing a pure strict op
into the branches of an abort-carrying `if` is value-preserving (the aborting branch never completes the op;
the other branch is unchanged), PROVIDED the op's other operands + the if-condition are PURE (else duplication
re-performs); (b) collapsing a strict frame whose operand aborts UNCONDITIONALLY to the abort value is correct
(strict eval order fires the abort first). The DANGER (proven, must be prevented): a naive per-branch capture
that threads the abort VALUE into the continuation instead of homing it → the 10223-vs-5113 (#7766) and
103-vs-99 (#7790) miscompiles. The safe floor (decline) is the fallback for any shape the fold does not model
— `thread_returning_tagged` returns `None` → the fold's overall `?` → CDZ0900. **Never widen the gate without a
matching threader arm; an unmodeled routed shape must decline, not mis-fold.**

## 5. A/B verification plan (per case, mandatory — soundness-critical)

For EACH case, in order, A/B every step:
1. Value: `cargo xtask gate --case "<title>" <chapter>` = the idealistic value on WASM.
2. Value on rust + rust-async (`--target rust`/`rust-async`) — the fold is backend-agnostic effects reduction,
   so it should pass all three (a rust CODEGEN build-fail is v-rust-backend's lane, route it — see #7750/#7753).
3. Leak: `--guarded-all --case "<title>"` = leak→0 (heap cases depend on v-core-opt's reclaim slices; a
   nested-tuple leak = route to v-core-opt like 13175→slice-3a #7765).
4. No regression: the resumptive controls (rw1/rw3/rwmatch) STAY pass; the naive-wrong shapes STAY declined.
5. Corpus-wide: in-process `cargo xtask gate --check <all effects chapters: 14 14b 14c 21 23 26>` = 0 regressed
   / 0 failing (the "N vanished" is a benign subset artifact — the baseline covers the whole corpus).
   ⚠ The full-corpus in-process gate EXCEEDS its 2700s wall-clock cap (useless); use the effects-chapters
   `--check` subset. The nix-cached `cargo xtask gate <files>` (no --case) builds from PINNED source, NOT the
   worktree — use `--check` (forces IN-PROCESS = the local build) or `--case`.
6. `cargo clippy -p rcdzc --all-targets` clean.
7. Baseline flip: co-land the `todo`→`pass` flip on ALL 3 baselines (keep the "declines cleanly" title — the
   12813/#7748 precedent; a title change would vanish the baseline line). 🪤 The UNION merge-driver DROPS the
   flip on rebase — re-apply before `rebase --continue`.
8. Land: self-merge PR to main (`gh pr create --base main`; mergeable/CLEAN; no CI reports on this repo).

## 6. Sequencing (fresh-session order)

Do the tractable/close ones first, hardest last, one landed PR per case (or per tight group):
1. **INDIRECT accum abort** (closest to #7790 — reuse the accum-off-tail machinery + helper inline).
2. **and-short-circuit** (the and/or→if desugar mostly exists).
3. **abx3 state-reading abort** (relax the perform-leaf state-ref guard + thread the state into the abort value).
4. **conditionally-resuming arm** (harder — compose resume + tagged; may split).
5. **foreign-perform-sibling / needs-frames** (assess feasibility; may stay declined if it needs real frames).
6. **glb1 collapse** (per its own sub-design; fold in or keep separate).
Multi-shot/Ty::Cont cases stay EXCLUDED.

## 7. Fresh-context reboot

Bank this doc + the vertical log's landed-CC entries. On reboot: read this design + `v-effects-*` memory log,
then start at sequencing step 1 (INDIRECT accum abort). The concierge offered to tmux-restart the window once
this design lands; otherwise self-manage. Do NOT carry this exploration context toward the wall — the design
is the hand-off.
