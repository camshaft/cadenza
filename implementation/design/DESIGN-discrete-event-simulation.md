# Design — discrete-event simulation (DES): spawnable tasks, `sleep(duration)`, a fast-forwarding virtual clock (port of camshaft/bach)

**Author:** design-des (interactive design agent, operator-directed 2026-07-18, relayed via
concierge→Slack). **Audience:** the operator (shaping this live); a future `vertical` owner
(area=`compiler-ml`, building the DES as an idiomatic Cadenza library); **v-effects** (owns the E5
general-continuation primitive this vertical is the forcing consumer of — see
`DESIGN-general-continuations-e5.md`); v-inference (the `Cont A B` type + effect rows); v-runtime
(the `Ty::Cont` frame-chain heap rep).
**Status:** 🟡 **IN DESIGN — API RESOLVED.** The two hard forks are LOCKED with the operator:
**one-shot resumable continuations** and **`UInt64`-nanosecond time in a newtype `Instant`**
(operator-ruled, §3.2). The bach API is now RESOLVED against the cloned repo
(`/local/home/bythewc/Projects/camshaft/bach`, §7) — bach is async/await, and `.await` maps 1:1 onto
Cadenza `perform`; `Sim` core = `sleep`/`spawn`(joinable)/`join`/`now`; sim runs until PRIMARY tasks
complete. Same-time tie-break confirmed **FIFO** (bach `push_back`/`pop_front`, §3.4). Partial-order
reduction / coop scheduling are **out of scope** (operator: "just get a task system in place"). Written
in the house style of `DESIGN-choreographic-protocols.md` and `DESIGN-effects-rcdzc.md` (vision →
increments → seams → gate → open decisions with a chosen default).

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
| `Instant`/`Duration` `UInt64`-ns newtypes + their ops | **this vertical** (over prelude `UInt64`) | The virtual clock's number type (§3.2). |
| Time-ordered event queue (priority queue keyed by `Instant`) | **this vertical** | Pure Cadenza over the value heap (Map/RRB). |
| `Sim` effect + scheduler handler + `spawn`/`sleep`/`now`/`run` surface (the bach port) | **this vertical** | The library the operator writes simulations against. |

**Why this ordering is safe:** v-effects builds E5 steps 1–2 (classifier+surface, within-activation
capture) *unblocked* — they prove the mechanism on self-contained cases. This vertical's scheduler is
the co-verification target for E5 **step 3** (stored/escaping `k` — the DES-critical step). So the DES
vertical starts by building everything that does NOT need step 3 (the queue, the `Instant` clock, the
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

### 3.2 Time is `UInt64` nanoseconds wrapped in a newtype `Instant` (OPERATOR-RULED 2026-07-18)

**Operator ruling (verbatim):** *"Use UInt64 for time in nanos. Make it a new type Instant so it is got
strong typing."* This supersedes an earlier `Qty`-units exploration — the DES clock is a plain integer
nanosecond counter with a nominal wrapper for type safety, matching bach's own representation
(bach `Instant` wraps a `Duration`, ns-resolution, `time.rs:31`):

- **`Instant`** — a **nominal newtype over `UInt64`** = nanoseconds since simulation start (`t0 = 0`).
  Strong typing: an `Instant` is not interchangeable with a raw `UInt64` or a `Duration`. Mirrors bach's
  `Instant`.
- **`Duration`** — a **nominal newtype over `UInt64`** = a span in nanoseconds. Distinct type from
  `Instant` (a point vs a span), so the type system rejects meaningless mixes.
- **Constructors** — `secs`/`ms`/`us`/`ns` build a `Duration` from a `UInt64` scaled to ns (mirrors
  bach's `DurationLiteral`: `5.s()` / `100.ms()` / `.us()` / `.ns()`, `ext.rs:10`). `(secs 5)` = a
  `Duration` of `5_000_000_000` ns.
- **Operations** (the only ways to combine them, enforcing the discipline):
  - `now : (-> Unit Instant)` — read the clock (§3.3).
  - `sleep : (-> Duration Unit)` — advance by a span.
  - `at : (-> Instant Duration Instant)` — `instant + duration` (wake-time computation).
  - `since : (-> Instant Instant Duration)` — `later − earlier`, a span (bach `Instant::elapsed`).
  - `before? : (-> Instant Instant Bool)` — event-queue ordering (plain `UInt64` `<` under the hood).

> **Why newtypes over the `Qty` units layer:** the operator wants strong typing + a minimal, shippable
> task system, not the full dimensional-units machinery. `UInt64` nanoseconds is exactly bach's model
> and gives O(1) integer comparison for the event queue. The `Instant`/`Duration` **nominal newtypes**
> (erased, zero-cost — `DESIGN-nominal-newtype-erasure-rcdzc.md`) provide the point-vs-span type safety
> the operator asked for without depending on the units subsystem. `UInt64` range: ~584 years of ns —
> ample for any simulation.

### 3.3 `now()` is observable to tasks (default; confirm vs bach in §7)

A task can read the current simulated time via the `Sim.now` op. This is standard DES (SimPy's
`env.now`) and near-certainly what bach exposes; final signature pinned when the operator pastes the
bach API.

### 3.4 Same-time tie-break — FIFO (CONFIRMED against bach)

When two events are scheduled at the *same* `Instant`, resume order must be deterministic. **FIFO —
confirmed against bach:** its timer stack uses `push_back`/`pop_front` (`time/entry.rs:132`), so
same-tick entries resume in insertion order. We match with a stable insertion-ordered secondary key in
the queue. Determinism here is load-bearing for the corpus gate (§8) — the expected event order must be
reproducible.

---

## 4. The surface (bach port — default shape, pending the operator's API)

The library exposes the `Sim` effect and a thin set of task-facing functions. This is the **bach-faithful
default**; §7 will reconcile names/signatures with the pasted bach API.

```
; The effect the scheduler discharges. Ops are what a task can do.
; NOTE (bach reconciliation, §7): bach is async/await — a task is an `async` block, and each `.await`
; on a sleep/join future IS the suspension point. In Cadenza that suspension point is `perform Sim.op`;
; the effect handler is the executor. So `sleep`/`spawn`/`join` map 1:1 onto `.await` points.
effect Sim {
  op sleep : (-> Duration Unit)         ; suspend until now + d; resumes exactly once (bach: d.sleep().await)
  op spawn : (-> (-> Unit A) (Task A))  ; enqueue a task; returns a joinable handle (bach: fut.spawn())
  op join  : (-> (Task A) A)            ; suspend until a spawned task finishes, yield its result (bach: handle.await)
  op now   : (-> Unit Instant)          ; read current simulated time (bach: Instant::now())
}

; Task-facing sugar (ordinary functions performing Sim ops):
(def (sleep d)   (Sim.sleep d))         ; bach: d.sleep().await
(def (spawn t)   (Sim.spawn t))         ; bach: (async t).spawn()  — returns a (Task A)
(def (join h)    (Sim.join h))          ; bach: handle.await
(def (now)       (Sim.now ()))          ; bach: Instant::now()
(def (sleep-until t) (sleep (since t (now))))   ; bach: sleep_until — derived; span from now TO t = (- t now) since `since later earlier` = later−earlier, so the target `t` is `later`

; Kick off a simulation. bach's `sim(f)` runs until all PRIMARY tasks complete (background tasks —
; e.g. a server loop — may run forever; the sim ends when the primaries are done, NOT when the queue
; drains). `main` is the initial primary task; `run-sim` returns main's result + final sim-time.
(def (run-sim main) ...)   ; installs the scheduler `handle`, seeds the ready-queue with `main` as primary
```

### 4.1 The scheduler = the `handle` block (the heart)

The scheduler is a `handle Sim` whose state is `(clock, event-queue, ready-queue)` and whose arms file
or enqueue continuations, then return control to a scheduler loop that pops the next event:

```
handle Sim (SchedState now0 empty-pqueue empty-ready)
  ( ; sleep binds k = the reified rest of the task; store it, do NOT resume now
    (sleep (d) s k
      (let ((waketime (at (clock-of s) d)))   ; at : Instant → Duration → Instant
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
; `secs`/`ms`/etc. are DES-library Duration constructors (UInt64 ns) — see note below.
(def (worker name delay)
  (do (sleep delay)
      (println (str name " woke at " (show (now))))))

(def (main)
  (do (spawn (fn () (worker "A" (secs 3))))
      (spawn (fn () (worker "B" (secs 1))))
      (sleep (secs 5))
      (println (str "main done at " (show (now))))))

(run-sim main)
; deterministic output (virtual time fast-forwards; no real waiting):
;   B woke at 1 s
;   A woke at 3 s
;   main done at 5 s
```

> **On the duration constructors:** a task builds spans via the DES-library constructors, mirroring
> bach's `DurationLiteral` (`5.s()`, `100.ms()`): `(def (secs n) (Duration (* n 1000000000)))`,
> `(def (ms n) (Duration (* n 1000000)))`, `(def (us n) (Duration (* n 1000)))`, `(def (ns n)
> (Duration n))` — each wraps a `UInt64` ns count in the `Duration` nominal newtype (§3.2). A task never
> handles a bare `UInt64`; it uses `secs`/`ms`/`us`/`ns`, so the `Instant`/`Duration` discipline holds.

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

1. **Time & queue substrate (no effects yet).** The `Instant`/`Duration` nominal newtypes over `UInt64`
   ns + the `secs`/`ms`/`us`/`ns` constructors + the ops `at`/`since`/`before?` (§3.2), a time-ordered
   priority queue over the value heap (insert / pop-min / FIFO secondary key ordered by `before?` on
   `Instant`), and a ready-queue. Pure library, unit-tested + a wasmtime run. Gate: fold unit + a
   value-executing run.
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
5. **The bach breadth.** bach's higher-level layers, each a sub-slice on top of the core (§7.3):
   `sync` channels / mpsc / mutex / semaphore / rwlock (§7.3), composable `queue`s (latency / loss /
   sojourn), `net` UDP simulation, `rand`/seeded interleaving. Scope + ordering per §7.3; v1 core is
   `sleep`/`spawn`/`join`/`now`.

---

## 7. The bach API surface — RESOLVED (repo studied at `/local/home/bythewc/Projects/camshaft/bach`)

The concierge cloned bach; §4 is now reconciled with its **actual** interface, not a default. The single
most important finding shapes the whole port:

### 7.1 bach is async/await — `.await` IS the continuation capture

bach models each logical process as a Rust `async` block; the executor (`bach::sim(f)`) is a
non-real-time reactor. Every `.await` on a bach future (`d.sleep().await`, `handle.await`, a channel
`recv().await`) is a **suspension point**: the executor parks the task's future and resumes it when the
awaited event fires. **This is exactly Cadenza's E5 story:** `perform Sim.sleep` is the suspension
point, the `handle Sim` block is the executor, and the reified continuation `k` is bach's parked
future. So the port is not an analogy — bach's `.await` and Cadenza's `perform` are the *same
mechanism*, which is why the operator adopted general continuations for this. A task author writes
straight-line effectful code; each `Sim` op that suspends is a `.await` in bach terms.

### 7.2 Core primitives (bach → Cadenza `Sim`), all in §4

| bach (Rust) | Cadenza `Sim` | Notes |
|-------------|---------------|-------|
| `d.sleep().await` / `time::sleep(d)` (`time.rs:15`) | `(sleep d)` → `perform Sim.sleep` | suspend until `now+d`; one-shot resume |
| `time::sleep_until(instant)` (`time.rs:25`) | `(sleep-until t)` = `(sleep (since t (now)))` | derived, not primitive; `since t (now)` = `t − now` (a future target `t` is `later`) |
| `fut.spawn()` → `JoinHandle<T>` (`ext.rs:62`, `task.rs:16`) | `(spawn t)` → `(Task A)` | **returns a joinable handle**, NOT fire-and-forget |
| `handle.await` → `Result<T, JoinError>` (`join.rs:37`) | `(join h)` | suspend until the task finishes, yield its result |
| `handle.abort()` (`join.rs:26`) | `(abort h)` (increment 5) | cancel a spawned task |
| `Instant::now()` (`time.rs:35`) | `(now)` → `Instant` | `Instant` = `UInt64` ns since sim start (§3.2) |
| `5.s()` / `100.ms()` / `.us()` / `.ns()` (`ext.rs:10` `DurationLiteral`) | `(secs 5)` / `(ms 100)` / … | `UInt64`-ns `Duration`-newtype constructors (§3.2, §4.2) |
| `fut.primary()` (`task.rs:62` `primary::Guard`) | primary vs background task | **termination:** see §7.4 |
| `bach::sim(f)` (`lib.rs:34`) | `(run-sim main)` | runs until primaries done |

### 7.3 Higher layers (increment 5, in bach's own module order)

bach's `sync` (channel / mpsc / duplex / mutex / rwlock / semaphore / queue), `queue` (latent / priority
/ sojourn), `net` (UDP sim + monitor + pcap), `rand` (seeded + `Any` interleaving), `coop`
(partial-order reduction). **All of these are built on the same suspend-on-`.await` core** — e.g. a
channel `recv` suspends the receiver until a sender wakes it, which is `perform Sim.recv` + the
scheduler resuming the receiver's `k`. So once the core (`sleep`/`spawn`/`join`) works, each layer is an
additive `Sim`-family op + a data structure, no new mechanism. Port order for increment 5: **(a)** mpsc
channel (the workhorse), **(b)** mutex/semaphore (queueing-theory demos, cf. `camshaft/kew`), **(c)**
`net` UDP (the README ping-pong). *(bach's seeded/POR interleaving — `rand`+`coop` — is OUT OF SCOPE
per the operator, §7.5; a potential follow-on vertical, not part of this DES.)*

### 7.4 Two semantics corrections from the real API (were wrong in the provisional default)

1. **spawn is joinable, not fire-and-forget.** `spawn` returns a `(Task A)` (bach `JoinHandle<T>`); a
   task can `(join h)` to suspend until it completes and read its result, or `(abort h)`. This adds a
   **`join` op** to `Sim` (§4) — itself a `sleep`-like suspension (the joiner's `k` is stored against
   the joinee's completion, not a wake-time). Same E5 mechanism, different wake trigger.
2. **The sim runs until PRIMARY tasks complete, not until the event queue drains.** bach ref-counts
   `.primary()` tasks via a `Guard` (`task.rs:85`); `sim` returns when the primary count hits zero.
   Background tasks (a server `loop {}`) may still be parked — that is *not* a hang, it is normal (the
   README's server task loops forever). So `run-sim` tracks a primary-count in scheduler state and
   terminates on zero-primaries, discarding still-parked background continuations. `main` is primary by
   default. (This also means "deadlock" = zero ready work AND a nonzero primary count with an empty
   timer queue — a detectable error state, cf. increment 5 diagnostics.)

### 7.5 Operator decisions (2026-07-18, via concierge) — all RESOLVED

The operator ruled on the three open items, all now baked into the design:

- **Time = `UInt64` nanoseconds in a newtype `Instant`** (verbatim: *"Use UInt64 for time in nanos.
  Make it a new type Instant so it is got strong typing."*). Matches bach's ns-resolution exactly;
  `Duration` likewise a `UInt64`-ns newtype. This replaced the earlier `Qty`-units exploration (§3.2).
- **primary/secondary IS needed** (verbatim: *"so we can distinguish between background tasks and ones
  that the sim cares about completing"*). Surface: `spawn` is **secondary/background** by default;
  `spawn-primary` (and `run-sim`'s `main`) is **primary**. Sim runs until the primary count hits zero
  (§7.4); secondary/background tasks do not hold the sim open.
- **Partial-order reduction / coop scheduling — OUT OF SCOPE for now** (verbatim: *"I do not care about
  partial order reduction/coop right now. I just want to get a task system in place."*). So bach's
  `rand`+`coop` interleaving (increment-5 item (d)) is **dropped from this vertical** — a potential
  follow-on vertical, not core DES. v1 target: *minimal + shippable* — `spawn`/`sleep`/`join`/`now` +
  primary/secondary + the fast-forward scheduler.

---

## 8. The gate that protects it

- **Increments 1–2:** `cargo test -p rcdzc --lib` (queue fold units + `Instant` time ordering + a wasmtime
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
- **2026-07-18 (tick 2)** — Sent v-effects the PROVISIONAL concrete `Sim` API + repro so their step 3
  is unblocked without waiting on the bach paste: both `sleep`/`spawn` capture `k : Cont Unit Ans`;
  store = `PQueue Time (Cont Unit Ans)` + FIFO ready-list; `resume` of a stored `k` = `apply(k, unit)`
  from the scheduler loop (their step-3 case). Only NAMES may shift when bach lands; shape is stable.
- **2026-07-18 (tick 3)** — **Qty time substrate VERIFIED** against the seed prelude (see §3.2): time
  dimension + literal sugar + same-dimension ordering/addition all exist today over `Int64`/`Float64`
  — increment 1 needs no fallback. Two corrections landed in this doc: (a) the literal surface is
  `5 seconds` / `(Qty.of …)`, **not** `(seconds N)` — added `secs`/`ms`/… DES constructors (§4.2); (b)
  the units layer has **no** instant/duration type split, so `Time`/`Duration` are DES nominal newtypes
  that enforce the discipline (§3.2). Bach API still pending from operator (slack-bridge ask out).
- **2026-07-18 (tick 4)** — **bach API RESOLVED** (concierge cloned the repo to
  `/local/home/bythewc/Projects/camshaft/bach`; studied `lib.rs`/`ext.rs`/`time.rs`/`task.rs`). Headline:
  **bach is async/await, `.await` == Cadenza `perform`** — the port is mechanism-identical, not an
  analogy (§7.1). Reconciled §4 + rewrote §7 from "pending default" to the real interface. Two semantics
  corrections (§7.4): (a) **spawn is joinable** (`JoinHandle`) not fire-and-forget → added a `join` op to
  `Sim`; (b) sim runs until **PRIMARY tasks complete**, not until the queue drains (background loops are
  fine) → `run-sim` tracks a primary-count. FIFO same-time tie-break CONFIRMED (bach `push_back`/
  `pop_front`, §3.4). Time = `Int64` **nanoseconds** to match bach's default tick (§7.5). Increment-5
  breadth mapped to bach's own module order (sync/queue/net/rand). v-effects unblocked + building E5
  steps 1-2 against the stable shape; their note confirmed they won't hardcode op names. `daadaa123` MR
  still queued; this reconciliation stacks as a further commit, held until `daadaa123` merges.
- **2026-07-18 (tick 5)** — `daadaa123` MERGED (trunk@`99be8ccbd`, later `30ee4f44b`). **Operator ruling
  (via concierge) supersedes the `Qty`-time decision:** time = **`UInt64` nanoseconds in a newtype
  `Instant`** (strong typing), `Duration` likewise; primary/secondary CONFIRMED needed; partial-order
  reduction/coop OUT OF SCOPE ("just get a task system in place — minimal + shippable"). Rewrote §3.2
  (Instant/Duration `UInt64`-ns newtypes + ops `at`/`since`/`before?`, replacing the `Qty` layer),
  propagated `Time`→`Instant` and dropped POR from increment 5, marked §7.5 all-resolved. v-effects also
  landed E5 **step-1 surface** (the `ctl`-style `k`-binding arm now parses/classifies, MR `e2d9fc60b`) —
  the surface the scheduler is written against EXISTS. Design is now essentially complete + operator-
  aligned; ready to squash the doc-refinement commits and hand to the PM once landed.
