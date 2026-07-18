# Design — discrete-event simulation (DES): spawnable tasks, `sleep(duration)`, a fast-forwarding virtual clock (port of camshaft/bach)

**Author:** design-des (interactive design agent, operator-directed 2026-07-18, relayed via
concierge→Slack). **Audience:** the operator (shaping this live); a future `vertical` owner
(area=`compiler-ml`, building the DES as an idiomatic Cadenza library); **v-effects** (owns the E5
general-continuation primitive this vertical is the forcing consumer of — see
`DESIGN-general-continuations-e5.md`); v-inference (the `Cont A B` type + effect rows); v-runtime
(the `Ty::Cont` frame-chain heap rep).
**Status:** 🟡 **IN DESIGN.** The two hard forks are LOCKED with the operator: **one-shot resumable
continuations** and **`Qty Duration` time** (§3). The remaining opens are the *bach API surface* (the
operator will paste it; §7 carries a bach-faithful default until then) and the same-time tie-break
(§3.4). Written in the house style of `DESIGN-choreographic-protocols.md` and `DESIGN-effects-rcdzc.md`
(vision → increments → seams → gate → open decisions with a chosen default).

> **The operator's directive (verbatim):** "the next vertical should be building a DES with spawnable
> tasks and the ability to sleep for a certain amount of time and have it fast forward. Basically we
> should be porting my camshaft/bach library to cadenza."

---

## 0. The one-sentence vision

**A simulation is a set of ordinary Cadenza tasks that `spawn` each other and `sleep` for a
`Duration`; a `sleep` does not block a wall clock — it captures the rest of the task as a one-shot
continuation, files it in a time-ordered queue keyed by `now + duration`, and yields to a scheduler
that fast-forwards the virtual clock straight to the next scheduled event — so a simulation of an hour
of behavior runs in microseconds, deterministically.**

The bet: a DES is *exactly* the forcing consumer that the effect system's general-continuation slot
(`DESIGN-effects-rcdzc.md` §4.4 — `Resume` is already an IR node, `Ty::Cont` reserved) was kept open
for. `sleep` = perform an effect that reifies "the rest of this task" as a first-class `Cont` value;
the scheduler = the `handle` block that *stores* that value and *resumes* it later. Nothing about the
DES needs new runtime primitives beyond the one-shot continuation v-effects is already building — the
scheduler, the event queue, and the clock are all pure Cadenza over the value heap. That makes the DES
both a real, useful library (the bach port) *and* the end-to-end proof that E5 continuations work.

---

## 1. What a DES is (and what it is NOT)

A **discrete-event simulation** models a system as a sequence of instantaneous *events* ordered on a
**virtual clock**. Between events, nothing happens, so the clock does not tick in real time — it
**jumps** ("fast-forwards") to the timestamp of the next event. This is the defining difference from a
real-time concurrency runtime:

- `sleep(5 seconds)` in a DES does **not** pause the process for 5 wall-clock seconds. It advances
  *simulated* time to `now + 5 seconds` — instantly, as far as the wall clock is concerned — by parking
  the task's continuation in the event queue at that timestamp and letting other ready work run first.
- Two tasks that each `sleep(1 hour)` and then act will both resume at simulated `1 hour`, in a
  well-defined order (§3.4), having consumed **zero** real time between them.
- When the event queue is empty, the simulation is over. `run` returns.

This is the SimPy / bach / des-style model, not the async/await-over-a-real-reactor model. The clock is
a value the scheduler owns, not a syscall.

---

## 2. The seam: DES = library-over-effects, forcing E5 continuations

The DES is **not** a compiler feature. It is a **Cadenza library** (the bach port) written against **one
new effect** — `Sim` — whose handler *is* the scheduler. The only compiler/runtime dependency is the
E5 general one-shot continuation, which v-effects owns and is already building (build order in
`DESIGN-general-continuations-e5.md` §5). The division of labor:

| Layer | Owner | What it provides |
|-------|-------|------------------|
| E5 continuation primitive (`ctl`-arm binds `k : Cont A B`, `resume k v = apply(k,v)`, stored/escaping `k`) | **v-effects** | The suspend/capture/store/resume mechanism. DES is the forcing consumer. |
| `Qty`/`Duration`/`Time` arithmetic + ordering | v-inference / prelude (exists) | The virtual clock's number type. |
| Time-ordered event queue (priority queue keyed by `Time`) | **this vertical** | Pure Cadenza over the value heap (Map/RRB). |
| `Sim` effect + scheduler handler + `spawn`/`sleep`/`now`/`run` surface (the bach port) | **this vertical** | The library the operator writes simulations against. |

**Why this ordering is safe:** v-effects builds E5 steps 1–2 (classifier+surface, within-activation
capture) *unblocked* — they prove the mechanism on self-contained cases. This vertical's scheduler is
the co-verification target for E5 **step 3** (stored/escaping `k` — the DES-critical step). So the DES
vertical starts by building everything that does NOT need step 3 (the queue, the `Qty` clock, the
`Sim` effect *declaration* and API shape, the corpus repro), and lands the live scheduler the moment
step 3 is green. See §6 for the increment interleave.

---

## 3. Locked decisions

### 3.1 One-shot resumable continuations (LOCKED)

Each `sleep` captures the current task's continuation `k` and resumes it **exactly once** — a task does
not fork by sleeping. This is E5's cheapest mode (RC-reclaimed on `apply`, affine-by-default, no
chain-copy) and needs **no** multi-shot opt-in. A second resume of the same `k` under the default is a
**compile-time reject** (`DESIGN-general-continuations-e5.md` §3). Multi-shot (fork-by-replay
simulation — e.g. speculative what-if branching) is explicitly **out of scope for v1** and noted as a
strictly-later opt-in.

### 3.2 Time is `Qty Duration` (LOCKED)

The virtual clock uses Cadenza's units-of-measure system, not bare integer ticks:

- `sleep : (-> Duration Unit)` — `Duration` is a `Qty` over a time dimension (e.g. `5 seconds`,
  `100 milliseconds`, `2 hours`). This gives type-safe durations for free and matches a units-aware
  bach.
- `now : (-> Unit Time)` — `Time` is a `Qty` *instant* on the same time dimension, measured from the
  simulation's `t0` (default `0 seconds`). Task-observable (§3.3).
- The event queue is keyed by `Time`, ordered by `Qty` comparison (already total on a single
  dimension). Adding a `Duration` to a `Time` yields a `Time` (`now + d`).

> **Substrate check owed before build:** confirm the prelude exposes a time dimension + `Qty` ordering
> and `Time + Duration : Time` add. If the `Qty` `Time`-vs-`Duration` distinction (affine-space-style:
> instant − instant = duration, instant + duration = instant) is not yet expressible, the first
> increment collapses to a single `Duration`-from-`t0` representation (a `Time` *is* a `Duration` since
> start) and we file the instant/duration distinction as a `Qty` follow-up. This keeps the DES
> unblocked on any current `Qty` limitation.

### 3.3 `now()` is observable to tasks (default; confirm vs bach in §7)

A task can read the current simulated time via the `Sim.now` op. This is standard DES (SimPy's
`env.now`) and near-certainly what bach exposes; final signature pinned when the operator pastes the
bach API.

### 3.4 Same-time tie-break — OPEN (default: FIFO by insertion order)

When two events are scheduled at the *same* `Time`, resume order must be deterministic. **Chosen
default: FIFO** — earlier-scheduled resumes first (a stable insertion-ordered secondary key in the
queue). Alternatives considered: LIFO, or an explicit priority argument on `sleep`/`spawn`. FIFO is the
least-surprising and matches most DES libraries; will confirm against bach. Determinism here is
load-bearing for the corpus gate (§8) — the expected event order must be reproducible.

---

## 4. The surface (bach port — default shape, pending the operator's API)

The library exposes the `Sim` effect and a thin set of task-facing functions. This is the **bach-faithful
default**; §7 will reconcile names/signatures with the pasted bach API.

```
; The effect the scheduler discharges. Ops are what a task can do.
effect Sim {
  op sleep : (-> Duration Unit)       ; suspend until now + d; resumes exactly once
  op spawn : (-> (-> Unit Unit) Unit) ; enqueue a new task (a thunk); runs on the scheduler
  op now   : (-> Unit Time)           ; read the current simulated time
}

; Task-facing sugar (ordinary functions performing Sim ops):
(def (sleep d)  (Sim.sleep d))
(def (spawn t)  (Sim.spawn t))
(def (now)      (Sim.now ()))

; Kick off a simulation: run `main` (and everything it spawns) until the event queue drains.
; Returns the final simulated time (and/or main's result — pinned vs bach in §7).
(def (run-sim main) ...)   ; installs the scheduler `handle`, seeds the queue with `main`
```

### 4.1 The scheduler = the `handle` block (the heart)

The scheduler is a `handle Sim` whose state is `(clock, event-queue, ready-queue)` and whose arms file
or enqueue continuations, then return control to a scheduler loop that pops the next event:

```
handle Sim (SchedState now0 empty-pqueue empty-ready)
  ( ; sleep binds k = the reified rest of the task; store it, do NOT resume now
    (sleep (d) s k
      (let ((waketime (time-add (clock-of s) d)))
        (scheduler-step (pqueue-insert s waketime k))))   ; return to loop

    ; spawn binds k = rest of the spawner; the new task is a fresh continuation from the thunk
    (spawn (t) s k
      (let ((s2 (ready-push s (thunk->cont t))))
        (scheduler-step (ready-push-cont s2 k))))          ; both spawner and child are ready now

    (now (u) s (resume (clock-of s) s)) )                  ; tail-resumptive: just read the clock
  (main))
```

`scheduler-step` is the fast-forward loop: if there is ready work, resume it; else pop the earliest
`(waketime, k)` from the pqueue, **set the clock to `waketime`** (the fast-forward), and resume `k`;
when both are empty, the simulation is done and `run-sim` returns. `resume k v = apply(k, v)` per E5 —
resuming a *stored* `k` from the scheduler loop is the same `apply` from wherever the queue holds it
(the DES-critical E5 step 3). Note `now` is a **tail-resumptive** arm (no `k` binder — it does not
suspend), so it stays cheap; only `sleep`/`spawn` are E5-general.

### 4.2 An example simulation the operator can read

```
(def (worker name delay)
  (do (sleep delay)
      (println (str name " woke at " (show (now))))))

(def (main)
  (do (spawn (fn () (worker "A" (seconds 3))))
      (spawn (fn () (worker "B" (seconds 1))))
      (sleep (seconds 5))
      (println (str "main done at " (show (now))))))

(run-sim main)
; deterministic output (virtual time fast-forwards; no real waiting):
;   B woke at 1 s
;   A woke at 3 s
;   main done at 5 s
```

---

## 5. Guarantees

- **Determinism.** Given the same program and the same same-time tie-break (§3.4), a simulation
  produces the identical event order and output every run — a hard requirement for the corpus gate and
  for a DES to be useful. No wall-clock, no OS scheduler, no nondeterminism.
- **Fast-forward = zero real time between events.** `sleep(1 hour)` costs a queue insert + a continuation
  capture, not an hour. Simulation cost is O(events), independent of simulated duration.
- **One-shot affine continuations (soundness).** Each task's `k` is resumed exactly once; the RC/fuel
  discipline of the value heap reclaims it on resume. No leaked frames, no double-resume.
- **In-program (no host span).** The scheduler and all tasks run in one program instance; a `sleep`
  continuation never spans a host call, so the host-composition invariant
  (`DESIGN-general-continuations-e5.md` §4) is satisfied by construction. CONFIRMED to v-effects.

---

## 6. Increments (top-to-bottom, the way a vertical lands them)

Each increment gates full-corpus + opt-sweep green before the next. Increments 1–3 do **not** need E5
step 3 (they build the pure-Cadenza substrate + the corpus repro); increment 4 lands the live scheduler
the moment v-effects' E5 step 3 is green, co-verified against increment 3's repro.

1. **Time & queue substrate (no effects yet).** The `Qty` `Time`/`Duration` layer (per §3.2, with the
   substrate-check fallback), a time-ordered priority queue over the value heap (insert / pop-min /
   FIFO secondary key), and a ready-queue. Pure library, unit-tested + a wasmtime run. Gate: fold unit +
   a value-executing run.
2. **`Sim` effect declaration + task API shape.** Declare `effect Sim` with `sleep`/`spawn`/`now`; the
   `sleep`/`spawn`/`now` task-facing wrappers; `now` as a tail-resumptive arm that works TODAY (no E5
   needed). A `sleep`/`spawn` handler arm compiles to the current clean E5 decline — no miscompile,
   just "not yet built" — until increment 4. This pins the exact surface v-effects builds `Cont A B`
   against.
3. **The corpus repro (the E5 step-3 gate).** A single end-to-end program: a 2-task interleave with time
   fast-forward and an expected, deterministic event order (the §4.2 example, value-graded). Handed to
   v-effects as the value-graded corpus case that gates their step 3. This is the contract between the
   two verticals.
4. **The live scheduler.** Once E5 step 3 (stored/escaping `k`) is green, land the `scheduler-step`
   fast-forward loop and `run-sim`; the increment-3 repro flips from `todo`→`pass`. This is the DES.
5. **The bach breadth.** Whatever inter-task communication + extras the pasted bach API defines
   (channels / mailboxes / `wait_for` / events / resources) — layered on top of the core, one sub-slice
   each. Scope pinned in §7 after the operator's paste.

---

## 7. Open decision — the bach API surface (default until the operator pastes it)

The operator will paste the bach core API (asked via slack-bridge). Until then, the design carries the
SimPy/bach-conventional **default** in §4 and this checklist of what the paste pins:

- **`spawn`** — does it return a *task handle* (joinable/awaitable), or is it fire-and-forget? Default:
  fire-and-forget thunk (`(-> (-> Unit Unit) Unit)`). If bach returns a handle you can `join`, that adds
  a `join`/`wait` op (itself a `sleep`-like suspension until the child completes) — additive to the
  same E5 mechanism.
- **`sleep`** — is there also `sleep_until(time)`? (Trivial: `sleep_until(t) = sleep(t - now())`.)
- **`run`** — return value: final sim-time, `main`'s result, both, or an event trace? Default: final
  `Time`.
- **inter-task comms** — channels / mailboxes / signals / `wait_for`? Or is v1 pure spawn+sleep?
  Default: **v1 is spawn+sleep+now only**; comms is increment 5, scoped to bach's actual primitives.
- **clock** — `now()` observable (default yes, §3.3); same-time tie-break (default FIFO, §3.4).
- **naming** — match bach's names (`spawn`/`sleep`/`run` vs whatever bach calls them) so the port reads
  as a port.

**Chosen default so the vertical is never blocked:** build increments 1–4 against the §4 shape; §5-comms
and any naming reconciliation fold in when the paste arrives (or, if the operator drops it, the §4 shape
*is* the design and increment 5 is a spawn+sleep+now-only v1).

---

## 8. The gate that protects it

- **Increments 1–2:** `cargo test -p rcdzc --lib` (queue fold units + `Qty` time ordering + a wasmtime
  run) + `cargo xtask gate` fail-set-diff clean + `cargo xtask check`.
- **Increment 3 (the repro):** a value-graded corpus case (deterministic event order) that reads `todo`
  until E5 step 3 lands — the shared contract with v-effects. A `todo`→`fail` flip there is a real
  miscompile (double-resume / wrong event order / clock not fast-forwarding); a `todo`→`pass` is E5
  step 3 landing correctly.
- **Increment 4+:** the repro flips to `pass`; add a 3-task same-time-tie-break case (guards §3.4
  determinism) and a `sleep(0)`/re-entrant-spawn edge case.
- **Cross-vertical:** every increment gated on current `trunk` after `fleet sync`; the corpus repro is
  co-owned with v-effects (they gate their E5 step 3 on it, I gate the scheduler on it).

---

## 9. Coordination log

- **2026-07-18** — Forks locked with operator: one-shot continuations (§3.1), `Qty Duration` time
  (§3.2). Asked operator for the bach core API via slack-bridge (spawn/sleep/run + comms + clock +
  same-time tie-break). v-effects landed `DESIGN-general-continuations-e5.md` (`7ae773912`); acked their
  §6 asks — confirmed one-shot + `Qty`-time + in-program (no host span); promised the exact `Sim`
  signature and the end-to-end repro (increment 3) once the bach API arrives. v-effects builds E5
  steps 1–2 unblocked now; step 3 co-verifies against this vertical's increment-3 repro.
