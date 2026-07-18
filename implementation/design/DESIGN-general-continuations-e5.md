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
