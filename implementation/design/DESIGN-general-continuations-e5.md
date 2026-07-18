# E5 general/stored effect continuations — the primitive for the DES vertical

Owner: v-effects. Status: DESIGN (operator-ruled REQUIRED 2026-07-18 — a discrete-event-simulation
vertical porting camshaft/bach needs stored, resumable continuations: a task that `sleep`s captures its
continuation, the scheduler stores it keyed by wake-time, and resumes it later when simulated time
advances). Co-design target: `design-des`. Builds directly on `DESIGN-effects-rcdzc.md` §4.4 (the slot was
kept open from day one: `Resume` is already a node, `Ty::Cont` is reserved).

## 1. What a DES needs from the continuation primitive

A minimal DES scheduler, expressed in Cadenza effects:

```
effect Sim {
  op sleep    : (-> Duration Unit)   ; suspend the current task until now+d
  op spawn    : (-> (Task Unit) Unit) ; enqueue a new task
  op now      : (-> Unit Time)        ; read simulated time
}
```

`sleep(d)` is the load-bearing op: performing it must **capture the rest of the current task as a value**,
hand it to the handler (the scheduler), which **stores** it in a time-ordered queue and returns control to
the scheduler loop. Later, when the loop pops that entry (simulated time fast-forwards to its wake-time),
it **resumes** the stored continuation, which runs the task's remainder until its next `sleep`/completion.

So the primitive the scheduler is written against is:

- **capture**: performing `sleep` reifies "the rest of this task" as a first-class `Cont` value `k`.
- **store**: `k` is an ordinary heap value — it goes in a `Map Time (List Cont)` / a priority queue.
- **resume-later**: `k` is invoked at an arbitrary later point, NOT lexically at the perform site, and NOT
  necessarily on the same handler activation — it is invoked from the scheduler loop.
- **one-shot suffices for a DES** (each `sleep` resumes its `k` exactly once — a task does not fork by
  sleeping). Multi-shot (fork-by-replay) is a strictly-later opt-in and NOT on the DES critical path.

## 2. Surface: `ctl`-style arm binding the continuation as a value

The tail surface `(op (params) s (resume value next-state))` keeps its implicit continuation. The general
surface binds `k` explicitly (per §4.4, "an arm may bind the continuation `k` as a value"):

```
handle Sim (scheduler-state)
  ( (sleep (d) s k   ; <- k is BOUND here: the reified continuation of the perform, a `Cont` value
      (let ((woken (Queue.insert s (+ (now-of s) d) k)))   ; store k keyed by wake-time
        (resume-scheduler woken)))                          ; return to the loop, do NOT resume k now
    (now  (u) s (resume (now-of s) s))
    (spawn (t) s k (let ((s2 (Queue.push-ready s (thunk->cont t)))) (resume-scheduler s2))) )
  (main-task))
```

- An arm's binder list gains an optional trailing `k` (the continuation), distinguishing a **general** arm
  (`k` bound) from a **tail-resumptive** arm (no `k`, uses `resume`). The classifier already computes this
  (§4.4 point 1/3): an arm that binds `k` — or uses `resume` non-tail — is E5-general.
- `resume` in a general arm is `apply(k, v)` — an ordinary application of the `Cont` value. `(resume v s)`
  stays sugar for the tail case (`k` used once, in tail position). Invoking a STORED `k` later is the same
  `apply(k, v)` from wherever the scheduler holds it.
- `Cont` is a first-class type (`Ty::Cont`). `k : Cont A B` = a suspended computation awaiting an `A` (the
  resume value) that, when applied, runs to a `B` (the handler's answer type). Storable in any collection;
  DES stores `List (Cont Unit Unit)` / `Map Time (Cont …)`. NOTE (correction, 2026-07-18): `Ty::Cont` is
  **not yet in the type enum** — it is added at STEP 3 (the stored/escaping-k case, which needs a heap rep
  for the frame-chain handle). STEP 2 (within-activation, `k` applied lexically once) does **not** need it:
  a lexically-applied `k` in a pure one-hole context is exactly the existing pure-continuation fold
  (`(k v)` = `C[v]`, the same term `resume` produces), so step 2 reuses that machinery with no new type.

## 3. Representation (from §4.4, made concrete)

Reify the delimited region perform→handler as a **defunctionalized frame chain on the frozen value-heap
prefix**:

- A frame = `sum-new(site-disc, arr-of-captured-locals)` — one variant per suspension site (each `sleep`
  perform site gets a disc), payload = the live locals the continuation needs.
- `k` = the frame-chain handle — an ordinary heap value of `Ty::Cont` (`core_valtype` = the heap-handle
  `I32`). This is WHY it stores + resumes later: it is just a heap value.
- `resume k v` = `apply(k, v)` where `apply` is ONE compiler-emitted `br_table` dispatcher (a fixed helper
  keyed on site-disc — control the flat rung can't express is a fixed helper, not a new instruction).
- **Envelope-neutral**: no new WIT op. Frames are `sum-new` over `arr`; `apply` is in-program. The DES
  scheduler is a pure-Cadenza program + the value-heap runtime — no host boundary.
- **One-shot** consumes the chain once (RC-reclaimed on apply). **Multi-shot** copies the chain per resume,
  behind a per-build opt-in (NOT needed for DES; a second resume under the default is a compile-time reject).

## 4. The host-composition invariant (a constraint the DES design must respect)

A reified continuation **must not span a host call**: `k` is a chain of run-local heap handles, which a
re-deriving host cannot reconstruct. Statically checkable from the effect-row classifier. IMPLICATION for
DES: the scheduler + tasks run in ONE program instance (in-program `Sim` handler); a task may not `sleep`
*across* a host-delegated effect boundary. This is naturally satisfied if the DES runtime is a Cadenza
library (the bach port) rather than a host-provided scheduler. **design-des: confirm this fits — is the
scheduler in-program (my assumption), or does any part delegate to a host clock/IO that a `sleep`
continuation would need to span?** If the latter, we need to discuss (the continuation cannot cross it).

## 5. Build order (v-effects owns; increments, each gated full-corpus + opt-sweep)

1. **Classifier + surface** (gate-neutral): recognize a `k`-binding / non-tail-resume arm as E5-general;
   parse the optional trailing `k` binder. A general arm still DECLINES cleanly to lower until step 3 — no
   miscompile, just "not yet built" (the current behavior, now with the surface in place).
2. **Frame capture + `apply` dispatcher** for the WITHIN-one-handle-activation general one-shot (non-tail
   resume, `k` applied lexically): the smallest real E5 — reify frames, emit the `br_table` `apply`, `resume`
   = `apply(k,v)`. Proves the mechanism on a self-contained case.
3. **Stored/escaping `k`**: `Ty::Cont` heap rep finalized, `k` storable in list/map, resumable from a
   DIFFERENT handler activation (the scheduler loop). This is the DES-critical step — co-verified against a
   minimal `sleep`/scheduler repro from design-des.
4. **(later, not DES)** multi-shot opt-in.

## 6. What I need from design-des to proceed

- The minimal `Sim`/scheduler API shape (ops + state) you are building — so step-3's `Cont` type params
  (`Cont A B`) and the store/resume signatures match what your scheduler holds.
- Confirmation of the in-program (no host-span) assumption in §4.
- A single end-to-end repro (`sleep`, `spawn`, a 2-task interleave with time fast-forward, expected event
  order) I can turn into the value-graded corpus case that gates step 3.

I will build steps 1–2 immediately (they are unblocked and prove the primitive); step 3 co-designs with your
scheduler shape. Sharing this now so you can design the scheduler against the `Cont` value + `apply`
semantics rather than waiting.

## 7. Step-3 scoping findings (v-effects, 2026-07-18) — TWO distinct escaping-k faces + forcing-consumer status

Scoping step 3 against the filed gates surfaced that "escaping k" is not one shape but TWO, with different
mechanisms — and that the current DES gate does NOT actually force step 3:

- **The DES 2-task gate ALREADY passes (→ 5000000000) via the tail-resumptive fold, NOT escaping-k.** Its
  `sleep`/`spawn` arms bind `k` but `(resume unit …)` IN PLACE (the pqueue/store/cross-activation-apply is
  in comments, not code); step-2's unused-k + let-peel fold it. So it is a fine SINGLE-activation gate but
  does not require the heap rep. A genuinely-storing repro (real pqueue in handler state, a scheduler-step
  that POPS + APPLIES a stored k) is needed to force step 3 — requested from v-discrete-event-sim.

- **FACE 1 — reified-continuation-k escape** (the `ctl`-surface case): `(op () s k (g k))` where `k` is
  passed to another function `g` and applied there (or stored in a data structure, applied cross-activation).
  Declines cleanly today. THIS is the case that needs the §3 `Ty::Cont` heap rep + defunctionalized frame
  chain + `apply(k,v)` `br_table` dispatcher — `k` genuinely leaves its activation as a value.

- **FACE 2 — captured-perform-RESULT escape** (the closure-capture tier-1 silent-value bug, `queue/adv-
  closure-captured-inner-handled-perform-reperforms-at-apply-under-outer.STEP3-GATED.sexp`): `base =
  (Ctr.tick)` under an inner handler, captured by a returned `(fn (x) (+ x base))`; applied under the OUTER
  handler → the perform RE-RUNS at apply (miscompiles to 8, want 53). This does NOT use the `ctl` k-surface
  — it is an ordinary `let`-bound perform captured by an escaping lambda. Its ROOT (diagnosed earlier) is a
  fold-vs-inline ORDERING problem: `(f 3)` β-reduces the returned closure BEFORE the inner handle folds
  `base` to its value, pulling the unfolded perform into the outer scope. Its fix is NOT necessarily the
  full `Ty::Cont` frame chain — it may be a **fold-before-inline barrier** (fold the inner handle's captured
  perform to a VALUE before the outer β-reduces the closure), which is a smaller (if delicate) change than
  the general reified-continuation machinery. TBD which; assess when building.

IMPLICATION: FACE 1 is the true `Ty::Cont` build (needs a genuinely-escaping-k forcing repro — the DES
storing-scheduler, requested). FACE 2 (the silent-value tier-1) may be separable via the ordering barrier
and is a valid forcing case on its own. Recommend: build the escaping-k heap rep (FACE 1) against a real
storing repro; assess FACE 2's ordering-barrier fix separately (it may not need the full frame chain). Do
NOT build the heap rep speculatively against the current DES gate (which folds tail-resumptively already).

## 8. Step-3 concrete build plan (v-effects, 2026-07-18) — GO, forcing repro in hand

FORCING CONSUMER (verified, no longer speculative): `queue/des-e5-step3-escaping-k-stored-apply.STEP3-
GATED.sexp` — the `sleep` arm hands `(wake, k)` to a SEPARATE top-level `scheduler-step` which applies
`(stored-k unit)` cross-activation. Declines cleanly today; expected `(: 5000000000 Int64)` when step 3
lands. `k` genuinely escapes (crosses a function boundary), so it CANNOT fold tail-resumptively.

The delimited continuation here: `main`'s body is `(do (Sim.sleep W) (inst-ns (Sim.now)))`; after `sleep`
the continuation `C` = `(inst-ns (Sim.now))` under the advanced state. `k` reifies "given resume value
`unit` + advanced state, run `C`". `scheduler-step` applies it.

BLAST RADIUS: a `Ty::Cont` variant touches ~15 files (every exhaustive `Ty` match — unify/infer/valtype/
both backends/lower/compile). So stage it; do NOT scatter a half-built variant.

INCREMENTS (each full-corpus + opt-sweep + rc-leak-probe gated):
1. **`Ty::Cont` variant, gate-neutral.** Add `Ty::Cont { resume: Box<Ty>, answer: Box<Ty> }` to the enum;
   give EVERY exhaustive match an arm that DECLINES cleanly (no runtime rep yet) — mirror how E2h-1 added
   `Core::HostCall` decline-arms everywhere. `core_valtype`/`valtype_of` = the heap-handle `I32` when built,
   `None`/decline until then. Gate-neutral: no program uses it yet, so 0 corpus change. This is the safe
   foundation slice (the memory trap: a new Ty variant needs a rust-backend arm + every match site).
2. **Reify the continuation as a frame value at the escaping-k perform.** When the fold sees an escaping `k`
   (the classifier's FACE-1 case — `k` passed to a fn / stored, not applied lexically), instead of
   declining, build the delimited continuation `C` as a defunctionalized frame: `sum-new(site-disc, arr-of-
   captured-locals)` (per §3). `k`'s value = the frame handle (`Ty::Cont`). The captured locals here = the
   handler state (the clock) — `C` reads `(Sim.now)` which needs the advanced state threaded into the frame.
3. **The `apply(k, v)` dispatcher.** `(stored-k v)` (ordinary application of a `Ty::Cont` value) lowers to a
   `br_table` over the site-disc that runs the corresponding `C` with `v` + the frame's captured locals.
   One compiler-emitted helper. This is where the DES repro's `(scheduler-step wake k)` → `(stored-k unit)`
   runs `(inst-ns (Sim.now))` under the advanced clock → 5000000000.
4. **Free-on-drop RC** (design-des's constraint): a `Ty::Cont` handle dropped un-applied frees its frame
   chain via Perceus RC. Gate with a live-objects probe (a captured-never-resumed k at teardown → 0 live).
5. **(later, not this repro)** multi-shot (copy the chain per apply, opt-in) + the multi-task pqueue (the
   DES run-sim: several `Ty::Cont` in a `Map Instant Cont`, popped + applied).

FACE 2 (closure-capture tier-1, the captured-perform-RESULT silent-value bug) is assessed SEPARATELY — it
may be a fold-before-inline barrier rather than the frame chain (see §7). Do it after the FACE-1 heap rep
exists (it may reuse the reify machinery, or the barrier may be simpler).

START: increment 1 (the gate-neutral `Ty::Cont` variant + decline-arms everywhere) next tick — the safe
foundation, then build up 2→3→4 against the DES escaping-k repro.

## 9. Increment-2 scoping breakthrough (v-effects, 2026-07-18): reuse the RUNTIME-CLOSURE lift

Scoping increment 2 against the IR revealed the reified continuation IS a CLOSURE — so it reuses the
existing runtime-closure machinery instead of a bespoke defunctionalized frame chain + `br_table`:

- `Core::Closure { code, captures }` (lower.rs `lower_lambda_value`, ~10086) already lifts a lambda
  `(fn (params) body)` into a heap closure CELL: slot 0 = `box-int(table-slot)`, remaining slots = the
  captured values. `Core::CallClosure { closure, args }` applies it at full arity via `call_indirect`.
- A reified continuation `k` at an escaping-k perform = the closure `(fn (resume-val) C)` where `C` is the
  DELIMITED CONTINUATION (the rest of the handle body after the perform), CAPTURING the live locals (the
  handler state at the perform). `apply(k, v)` = `Core::CallClosure { closure: k, args: [v] }` — the SAME
  `call_indirect` runtime-closure path, no new dispatcher.
- So `Ty::Cont { resume, answer }` ≈ a `Ty::Fn(resume, answer)` closure at runtime (both are i32 heap-cell
  handles — which is why increment 1 set `valtype_of(Cont) = i32`, matching `Ty::Fn`). The distinct `Ty::Cont`
  type keeps the CLASSIFIER honest (a continuation vs an ordinary function) but the RUNTIME rep is a closure.

REVISED increment plan (cheaper + lower-risk than the §3 frame chain):
2. At an escaping-k perform, the fold SYNTHESIZES the continuation lambda `(fn (#kv) C)` — `C` = the handle
   body with the perform replaced by `#kv` (the resume value hole), the handler state threaded to the
   perform's point captured as a free var — and lowers it via the existing `lower_lambda_value` → a
   `Core::Closure`. That closure value IS `k`, substituted for the arm's `k` binder. The arm body (e.g.
   `(scheduler-step wake k)`) then carries `k` as an ordinary closure value.
3. `(k v)` / `(stored-k v)` (ordinary application of the `Ty::Cont` value) lowers to `Core::CallClosure` —
   already handled by the runtime-closure application path (a fn-typed value applied). May need only to
   admit a `Ty::Cont`-typed head where the closure-apply path currently expects `Ty::Fn`.
4. Free-on-drop RC: a `Core::Closure` cell already participates in Perceus RC (the closure-leak probes
   exist) — so free-on-drop-un-applied may come largely for free; verify with a live-objects probe.

RISK: the continuation `C` must capture the handler STATE correctly (the DES `sleep` resumes with the clock
ADVANCED — `C = (inst-ns (Sim.now))` must see `wake`, not the pre-sleep state). And the host-composition
invariant (no host call in `C`) must be checked. But the heavy lifting (heap cell, call_indirect, RC) is the
existing closure path. This is a MUCH smaller increment 2 than a bespoke frame chain — validate the reuse
holds (the closure lift must accept a synthesized continuation lambda + the state capture) before committing.

## 10. Increment-2 feasibility CONFIRMED + build entry (v-effects, 2026-07-18)

The two building blocks the closure-reuse plan (§9) needs BOTH already exist and compose:
- **`C` extraction**: `splice_context(db, handle_body, perform, filler)` (effects.rs ~5441) returns
  `handle_body[perform := filler]` — exactly the delimited continuation with a hole. Used today by the
  two-hole refold. For reification, `filler` = a fresh `#kv` resume-value binder.
- **The lift**: `lower_lambda_value(db, id, params, body)` (lower.rs ~10086) lifts a `(fn (params) body)`
  into a `Core::Closure { code, captures }`, capturing free vars automatically. For reification, lift
  `(fn (#kv) C)`.

So the reification is: `C = splice_context(handle_body, perform, #kv)`; synthesize `(fn (#kv) C)`;
`lower_lambda_value` → `Core::Closure` = `k`. Substitute that `k` for the arm's `cont` binder; the arm body
(`(scheduler-step wake k)`) then carries `k` as a closure value; `(stored-k unit)` → `Core::CallClosure`.

BUILD ENTRY (the exact locus): `reduce_handle`'s classifier gate — the `None => return None` escaping-k
decline after `ctl_arm_lexical_k_to_resume` (effects.rs ~1622). Increment 2 replaces that `return None`
with the reification for the FACE-1 escaping-k arm (a `k` passed onward / stored, not lexically applied).

STATE-CAPTURE-TIMING (the concierge's flagged risk, to validate in the build): `C` must see the handler
state at the RESUMED (advanced) value. In the DES repro the arm resumes with `wake` (the fast-forwarded
clock), and `C = (inst-ns (Sim.now))` must read `wake`. So the synthesized continuation lambda must capture
the state as the arm computed it at the resume point — NOT the pre-perform state. When the arm applies `k`
(via `scheduler-step`'s `(stored-k unit)`), the state the arm threads (`(scheduler-step wake k)` passes
`wake`) must be what `C`'s `(Sim.now)` reads. Concretely: the `now` arm reads the state in place, so `C`'s
`(Sim.now)` resolves against whatever state is live when `apply(k, unit)` runs — which the scheduler-step
sets via the arm's own logic. This is the part to get exactly right + gate on the repro → 5000000000.

HOST-COMPOSITION INVARIANT: before reifying, check `C` reaches no host call (a reified continuation must not
span a host boundary). Statically checkable (the escaping-k arm's `C` is in-program in the DES repro).

Increment 2 is a focused multi-part build (synthesis + state capture + the fold gate) — start fresh, gate on
the DES escaping-k repro (must still DECLINE until inc 3 wires `apply`, then run to 5e9), full-corpus +
opt-sweep + rc-leak each slice.

## 11. Increment-2 REFINEMENT (v-effects, 2026-07-18): pure-C vs re-performing-C escaping-k

Probing the DES repro's continuation revealed inc 2 is TWO sub-cases of different difficulty — and the DES
repro is the HARDER one, NOT the minimal inc-2:

- **INC-2a — escaping-k with a PURE continuation `C`** (no re-perform of the handled effect in `C`):
  `(f () s k (use-k k))` over `(+ 1 (A.f))` — here `C = (+ 1 □)`, pure. Reifying `k` = the closure
  `(fn (#kv) (+ 1 #kv))`, no captures beyond the pure context, NO handler re-entry when applied. `apply(k,
  10)` = `(+ 1 10)` = 11. This IS the minimal escaping-k: a plain `Core::Closure` over a pure `C`, applied
  by the ordinary `Core::CallClosure`. The tractable first real-emit slice.

- **INC-2b — escaping-k whose `C` RE-PERFORMS the handled effect** (the DES repro): `C = (do □ (inst-ns
  (Sim.now)))` CONTAINS a `Sim.now` perform of the SAME effect. So applying `k` must RE-ENTER the handler
  (the `now` in `C` needs handling, under the ADVANCED state). The reified closure cannot be a plain pure
  closure — it must either carry the handler (fold `C` under the handler at apply) or the apply must
  re-fold `C` (like the two-hole `rewrite_resume_to_refolded_context`, but cross-activation). This is the
  DES case and is materially harder than 2a — it is the true "stored continuation re-enters its handler"
  capability. Gate: the DES repro → 5e9.

REVISED sequencing: build INC-2a (pure-C escaping-k → a plain closure) FIRST — it is bounded, reuses
`splice_context` + `lower_lambda_value` + `Core::CallClosure` cleanly, and proves the reification mechanism
on a self-contained case (a pure-C escaping-k → 11). THEN INC-2b (re-performing-C, the DES case) adds the
handler-re-entry-at-apply — the harder capability, gated on the DES repro → 5e9. This split keeps each slice
bounded + validatable (2a has no handler-re-entry subtlety; 2b isolates exactly that). Do NOT try to build
the DES case directly as "inc 2" — it conflates the closure reification (2a) with handler re-entry (2b).

Need a pure-C escaping-k CORPUS repro for 2a (I can author one: `(f () s k (use-k k))` over `(+ 1 (A.f))`
→ 11). The DES repro stays the 2b gate.

## 12. Increment-2a build hook (v-effects, 2026-07-18): extend the pure-one-hole block

The reification for INC-2a (pure-C escaping-k) hooks into the EXISTING pure-one-hole fold block
(effects.rs ~1816, `if let PureHole::Hole(perform) = pure_hole(db, body, &ctx)`), which ALREADY:
- finds the single discharged perform `P` in `body` (`pure_hole`), and
- has `splice_context(db, body, P, filler)` to build `C = body[P:=filler]`.

Today that block requires the arm to be TAIL-RESUMPTIVE (it rewrites `resume`→`C[v]`). Extend it: when the
arm binds `k` (escaping, `cont: Some`) AND `C` is pure, instead reify `k` = the closure `(fn (#kv) C')`
where `C' = splice_context(body, P, #kv)`, lower it via `lower_lambda_value` → `Core::Closure`, and
SUBSTITUTE that closure for the arm's `cont` binder in the arm body — then the arm body (`(use-k k)`)
carries `k` as a closure value, and `(k v)` / `(use-k)`'s `(stored-k v)` lowers to `Core::CallClosure`.

GATE RESTRUCTURING NEEDED: the classifier gate (`None => return None` at ~1622) currently DECLINES escaping-k
before reaching the pure-one-hole block. So route a pure-C escaping-k arm THROUGH (don't decline it there) —
carry `cont: Some` into `ctx.arms` for the escaping case, and let the pure-one-hole block reify it. A
re-performing-C escaping-k (inc-2b, the DES case) still declines at the block (pure_hole won't fire — `C`
has a second perform) until inc-2b adds handler-re-entry-at-apply.

BUILD ORDER within inc 2: (2a-i) route pure-C escaping-k past the classifier decline into the pure-one-hole
block [gate-neutral-ish: still declines if the block can't serve]; (2a-ii) reify `k` as the closure +
substitute for `cont`; (2a-iii) `(k v)` → CallClosure (admit a Ty::Cont head). Gate: `(f () s k (use-k k))`
over `(+ 1 (A.f))` → 11. Each micro-step full-corpus + opt-sweep. Start fresh — this is interlocking
fold+reification wiring where a wrong hook miscompiles, so build stepwise with per-step validation.

## 13. Increment-2b scoping (v-effects, 2026-07-18): re-performing-C escaping-k = bake the refold into the closure

inc-2a reifies an escaping k over a PURE C as `(fn (#kv) C)` — a plain closure. inc-2b is the crux: C
RE-PERFORMS the handled effect (DES: `C = (do □ (inst-ns (Sim.now)))` reads `(Sim.now)`; my `/tmp/2b.sexp`:
`C = (+ □ (St.tick))`). Applying the reified closure must RE-ENTER the handler (the inner perform in C needs
handling under the state at apply time) — but k escapes to a fn where the handler isn't lexically present.

MECHANISM (confirmed present): the TWO-HOLE refold `rewrite_resume_to_refolded_context` ALREADY folds a
re-performing continuation under the handler — `(+ (St.tick) (St.tick))` with a resume arm → 1 (it re-folds
C[v] under the handler via `reduce_handle(next_state, arms, C[v])`). So the machinery to handle a
re-performing C exists; inc-2b must BAKE that refold into the reified closure body.

inc-2b reification = `k = (fn (#kv) <C refolded under the handler, with #kv the resume value>)` — i.e. the
closure body is NOT the raw C but the RESULT of folding C[#kv] under the handler (so the inner perform is
already handled), with the handler STATE threaded through. The hard part: the state at apply time. For a
re-performing C the state ADVANCES (the DES sleep resumes with the clock fast-forwarded); so the reified
closure must thread the state — either (a) the closure captures the state and the refold threads it as a
closure-local, or (b) the state is a second closure param `(fn (#kv #state) …)`. The DES `sleep` arm's
`scheduler-step` sets the clock then applies k — so the state at apply is the scheduler's, passed IN. This
suggests the reified continuation is `(fn (#kv) …)` where the state it reads is captured from the ARM's
resume logic (the arm computes the advanced state + applies k). VALIDATE against the DES repro: the arm
`(sleep (wake) s k (scheduler-step wake k))` — scheduler-step applies `(stored-k unit)`; the reified k must,
when applied, read the clock as `wake` (the arm passed `wake` to scheduler-step, which sets clock:=wake).

This is the deepest slice — the state-capture-at-advanced-value + refold-in-closure interaction. Build it
carefully next tick: reuse `rewrite_resume_to_refolded_context` to fold C[#kv] under the handler as the
closure body; thread the state; gate on /tmp/2b.sexp (→ 0+1=1? recompute) + the DES repro → 5e9 + the
no-host-span invariant. It likely SUBSUMES v-cad's two-hole PRNG gap (same re-performing-C shape).

## 14. Increment-2b semantic finding (v-effects, 2026-07-18): the reified k captures the ARM's NEW state

Spiking the DES escaping-k repro surfaced the crux subtlety. The sleep arm is
`(sleep (wake) s k (scheduler-step wake k))`: it binds the NEW state as `wake` (the op's arg), ESCAPES `k`
to `scheduler-step` WITHOUT resuming, and scheduler-step applies `(stored-k unit)`. The continuation
`C = (inst-ns (Sim.now))` re-performs `Sim.now`, which must read `clock = wake = 5e9`.

KEY: the arm does NOT `resume` — it discards the incoming state `s` and hands `wake` + `k` to scheduler-step.
So the state the reified `k` must read when applied is NOT `s` (the pre-perform state) and NOT threaded via
`resume` — it is `wake`, which the ARM computes and (implicitly, by the DES contract) is the state at which
the continuation resumes. In this repro `wake` is the sleep op's ARGUMENT; the scheduler applies k "at" wake.

So the reified continuation closure must capture/read the state AS THE ARM SET IT. For the DES sleep,
`wake` is the op arg the arm threads to scheduler-step; the reified `k`'s `(Sim.now)` must resolve against
`wake`. Mechanically: reify `k = (fn (#kv) C-refolded-with-state=wake)` where the state the refold seeds is
the arm's new-state expression. But the arm here doesn't NAME its new state via `resume` — it's implicit in
`(scheduler-step wake k)`. This is subtler than a `resume`-based next-state: the DES contract is "the
scheduler applies k at the wake time it stored". So the reified k likely needs the state as a CLOSURE PARAM
(`(fn (#kv #state) C)`) that the scheduler supplies — OR the `now` op reads a scheduler-maintained clock
passed at apply. NEEDS co-design with v-discrete-event-sim on the exact clock-at-apply contract (does the
scheduler pass the wake-state when it applies k, or does k close over it?).

REVISED inc-2b plan: (1) the SIMPLE re-performing-C where the arm RESUMES (the two-hole shape, state via
resume's next-state) — bake the refold into the closure, state from the resume next-state (tractable, +
subsumes v-cad's PRNG which DOES resume). (2) the DES sleep shape (arm escapes k WITHOUT resume, state
implicit) — needs the clock-at-apply contract clarified with v-discrete-event-sim. Do (1) first (it's the
v-cad PRNG unblock + a clean state source); co-design (2) with the DES PM. FLAG the DES PM: your escaping-k
repro's sleep arm doesn't `resume` — how does the reified k learn the wake-state? Is it a closure param the
scheduler supplies at apply, or does `now` read a scheduler clock?
