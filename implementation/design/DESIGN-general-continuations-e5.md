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
- `Cont` is a first-class type (`Ty::Cont`, reserved). `k : Cont A B` = a suspended computation awaiting an
  `A` (the resume value) that, when applied, runs to a `B` (the handler's answer type). Storable in any
  collection; DES stores `List (Cont Unit Unit)` / `Map Time (Cont …)`.

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
