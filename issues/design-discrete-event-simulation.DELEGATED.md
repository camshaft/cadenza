# Vertical-ready brief — discrete-event simulation (DES), port of camshaft/bach

**Status:** design LANDED on trunk (`8cf884e1d`, base `daadaa123`) — ready for a `vertical` owner.

**Design doc:** `implementation/design/DESIGN-discrete-event-simulation.md` (fully on trunk).
**Subsystem / area:** `compiler-ml` — the DES is an idiomatic Cadenza LIBRARY over a `Sim` effect, not a compiler feature.
**bach source:** cloned at `/local/home/bythewc/Projects/camshaft/bach` — study `lib.rs`/`ext.rs`/`time.rs`/`task.rs`. KEY insight: bach is async/await and each `.await` IS the suspension point == Cadenza `perform Sim.op`, so the port is mechanism-identical.
**Depends on:** v-effects' E5 general one-shot continuations (`DESIGN-general-continuations-e5.md`, `7ae773912`; step-1 `ctl`-arm surface landed `e2d9fc60b`, building steps 2-3). Increments 1–3 do NOT need E5 step 3; increment 4 (the live scheduler) lands on E5 step 3 (stored/escaping `k`), co-verified against this vertical's increment-3 repro.

## What to build (first increment first)
1. **Time & queue substrate** (no effects): `Instant`/`Duration` nominal newtypes over `UInt64` nanoseconds (operator-ruled — strong typing, NOT the Qty units layer), constructors `secs`/`ms`/`us`/`ns`, ops `at`/`since`/`before?`; a time-ordered priority queue keyed by `Instant` (FIFO same-time tie-break); a ready-queue. Unit-tested + a wasmtime run.
2. **`Sim` effect declaration + task API shape:** `sleep`/`spawn`(joinable→`Task A`)/`join`/`now`. `now` is a tail-resumptive arm (works today); `sleep`/`spawn`/`join` are E5-general (decline cleanly until step 3).
3. **The increment-3 corpus repro** (the shared gate with v-effects): the 2-task interleave below, value-graded on event ORDER + final time. v-effects gates their E5 step 3 on it.
4. **The live scheduler + `run-sim`** — once E5 step 3 is green; runs until PRIMARY tasks complete (primary/secondary ref-count), not until the queue drains.
5. **bach breadth:** sync (mpsc channel first) / queue / net UDP. (Partial-order-reduction / coop OUT of scope per operator.)

## The increment-3 repro (final surface)
```
(def (worker name delay)
  (do (sleep delay)
      (emit (str name " woke at " (show (now))))))
(def (main)
  (do (spawn (fn () (worker "A" (secs 3))))
      (spawn (fn () (worker "B" (secs 1))))
      (sleep (secs 5))
      (emit (str "main done at " (show (now))))))
(run-sim main)
```
Expected (deterministic, FIFO same-time): `B woke at 1s`, `A woke at 3s`, `main done at 5s`; final sim-time 5s. No real time elapses between events.

## Coordination notes for the owner
- v-effects owns the continuation primitive and has ACCEPTED the full contract: `join` = same `Cont A Ans` (parametric over resume type); drop-unresumed (a parked background-task `k` at sim end) frees via Perceus RC on drop-without-apply, gated by a live-objects leak probe; `Instant` store key. Coordinate step-3 landing against the repro above; ping v-effects when increment-3 lands.
- Time is `UInt64` ns internally; the vertical owns ns→human formatting for `show`.
- Design agent `design-des` stands down on handoff; conversation scrollback is in its tmux window if you need the design rationale.
