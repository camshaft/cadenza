; Discrete-event simulation (DES) — port of camshaft/bach as an idiomatic Cadenza LIBRARY.
;
; Design: implementation/design/DESIGN-discrete-event-simulation.md. A DES is a set of ordinary
; Cadenza tasks that `spawn` each other and `sleep(Duration)`; a `sleep` captures the rest of the
; task as a one-shot continuation, files it in a time-ordered queue keyed by `now + duration`, and
; yields to a scheduler that FAST-FORWARDS the virtual clock straight to the next event. The
; scheduler is a `handle Sim` block; the event queue + clock are pure Cadenza over the value heap.
;
; The DES lands in gated increments (design §6). This file grows one increment at a time:
;   INCREMENT 1 (below) — the PURE SUBSTRATE, no effects: the `Instant`/`Duration` nominal newtypes
;     over `UInt64` nanoseconds (operator-ruled §3.2: strong typing, NOT the Qty units layer), their
;     constructors (`secs`/`ms`/`us`/`ns`) and ops (`at`/`since`/`before?`), plus a time-ordered
;     priority queue (insert / pop-min / FIFO same-time tie-break) and a ready-queue. Buildable and
;     gated TODAY — needs no continuations.
;   INCREMENT 2 (next) — the `Sim` effect declaration + task API shape; `now` tail-resumptive.
;   INCREMENT 3 — the 2-task-interleave corpus repro (the shared gate with v-effects' E5 step 3).
;   INCREMENT 4 — the live fast-forward scheduler + `run-sim`.
;
; Every case here is self-contained (its own `type`/`def`s) so the corpus reader needs no library
; import machinery — the DES library is a set of ordinary defs, faithfully reproduced per case.

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 1 — Instant / Duration newtypes over UInt64 nanoseconds (§3.2)
; ────────────────────────────────────────────────────────────────────────────────────────────────

(case "a Duration constructor `secs` scales a UInt64 count to nanoseconds"
  (doc    "`(secs 5)` builds a `Duration` of 5_000_000_000 ns — the bach `5.s()` DurationLiteral
           (ext.rs:10), scaled by 1e9. `Duration` is a nominal newtype over `UInt64` (§3.2), so the
           returned value prints as `(: 5000000000 Duration)` — the ns count with the nominal name. This
           is the base unit-scaling the whole clock rests on: a task never handles a bare `UInt64`, only
           `secs`/`ms`/`us`/`ns`, so the Duration discipline holds by construction.")
  (input  (do
            (type Duration (Duration UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (main) (secs 5))
            (export main)))
  (output (: 5000000000 Duration)))

(case "the DES Duration constructors ms / us / ns each scale to the right nanosecond magnitude"
  (doc    "`ms`/`us`/`ns` mirror bach's `100.ms()` / `.us()` / `.ns()` (ext.rs:10): `(ms 100)` =
           100_000_000 ns, `(us 100)` = 100_000 ns, `(ns 100)` = 100 ns. Pins each constructor's scale
           factor (1e6 / 1e3 / 1) so a future edit can't silently transpose two of them — a wrong scale
           would make every sleep in the wrong unit compile clean yet run wrong. Runs each via a boundary
           `(call …)` arg (a runtime UInt64 that cannot fold) so the multiply executes as a real
           instruction, then unwraps to the ns count.")
  (input  (do
            (type Duration (Duration UInt64))
            (def (ms (: n UInt64)) (Duration.Duration (* n 1000000)))
            (def (us (: n UInt64)) (Duration.Duration (* n 1000)))
            (def (ns (: n UInt64)) (Duration.Duration n))
            (def (dur-ns (: d Duration)) (match d ((Duration.Duration v) v)))
            (def (main (: kind UInt64) (: n UInt64))
              (if (= kind 0) (dur-ns (ms n))
                  (if (= kind 1) (dur-ns (us n)) (dur-ns (ns n)))))
            (export main)))
  (call   main (: 0 UInt64) (: 100 UInt64))
  (output (: 100000000 UInt64))
  (call   main (: 1 UInt64) (: 100 UInt64))
  (output (: 100000 UInt64))
  (call   main (: 2 UInt64) (: 100 UInt64))
  (output (: 100 UInt64)))

(case "`at` advances an Instant by a Duration (wake-time computation)"
  (doc    "`(at t d)` = `t + d` — the scheduler's wake-time computation (design §3.2, §4.1: the sleep
           arm files a continuation at `(at (clock-of s) d)`). `Instant`/`Duration` are distinct nominal
           newtypes over `UInt64`; `at` unwraps both, adds the ns, and re-wraps as an `Instant`. From
           `t0 = 0` and a 3-second span, the wake Instant is 3_000_000_000 ns. This is the only way a
           point advances by a span, so it pins the point-plus-span arithmetic the event queue keys on.")
  (input  (do
            (type Duration (Duration UInt64))
            (type Instant (Instant UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
            (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
            (def (main) (at (Instant.Instant 0) (secs 3)))
            (export main)))
  (output (: 3000000000 Instant)))

(case "`since` is the span between two Instants (later minus earlier)"
  (doc    "`(since later earlier)` = `later − earlier`, a `Duration` — bach's `Instant::elapsed`
           (time.rs). It is the dual of `at`: `at` adds a span to a point, `since` subtracts two points
           to a span. `(since t3 t0)` where t3 is 3 s and t0 is 0 yields a 3-second `Duration`
           (3_000_000_000 ns). This is what `sleep-until` is derived from — `(sleep-until t)` =
           `(sleep (since (now) t))` (§4) — so the span-from-now-to-a-target computation is pinned.")
  (input  (do
            (type Duration (Duration UInt64))
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (at (: t Instant) (: d Duration))
              (Instant.Instant (+ (inst-ns t) (match d ((Duration.Duration n) n)))))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (since (: later Instant) (: earlier Instant))
              (Duration.Duration (- (inst-ns later) (inst-ns earlier))))
            (def (main)
              (let ((t0 (Instant.Instant 0))
                    (t3 (at (Instant.Instant 0) (secs 3))))
                (since t3 t0)))
            (export main)))
  (output (: 3000000000 Duration)))

(case "`before?` orders two Instants (the event-queue comparison)"
  (doc    "`(before? a b)` = the underlying `UInt64` `<` — the ONLY comparison the time-ordered event
           queue uses to sort wake-times (design §3.2, §4.1). `1 ns` is before `3 ns` (true); the strict
           `<` means an Instant is NOT before itself (the same-time case is a tie-break, §3.4, not a
           before?-true — pinned in the next case). Determinism of the whole sim rests on this being a
           total strict order over the ns counter.")
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
            (def (main) (before? (Instant.Instant 1) (Instant.Instant 3)))
            (export main)))
  (output (: true Bool)))

(case "`before?` is a STRICT order — an Instant is not before an equal Instant (same-time is a tie-break)"
  (doc    "The same-time boundary: `(before? t t)` is FALSE for equal Instants. This is load-bearing for
           the FIFO same-time tie-break (§3.4) — two events at the SAME instant are NOT ordered by
           `before?`; they resume in INSERTION order, which the queue realizes by inserting a new
           equal-key entry AFTER the existing equal-key ones (the `q-insert` cases below). A `<=` here
           instead of `<` would break FIFO by making a later same-time insert compare 'before' an earlier
           one. Both same-time Instants are 1 s.")
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
            (def (main) (before? (Instant.Instant 1000000000) (Instant.Instant 1000000000)))
            (export main)))
  (output (: false Bool)))

(case "an Instant and a Duration are DISTINCT nominal types — a point cannot be used where a span is expected"
  (doc    "The strong-typing the operator asked for (§3.2, verbatim 'strong typing'): `Instant` and
           `Duration` both erase to `UInt64`, but they are DISTINCT nominal newtypes — a point vs a span.
           `at` expects `(: d Duration)` as its second argument; passing an `Instant` there is rejected
           CDZ0203 naming both nominal types, even though both erase to `UInt64`. This pins the
           point/span type safety: you cannot accidentally add two Instants or sleep for an Instant. The
           program's outcome is the rejection.")
  (input  (do
            (type Duration (Duration UInt64))
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
            (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
            (def (main) (at (Instant.Instant 0) (Instant.Instant 3)))
            (export main)))
  (error  CDZ0203))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 1 — the time-ordered event queue (priority queue, FIFO same-time tie-break, §3.4, §4.1)
; ────────────────────────────────────────────────────────────────────────────────────────────────

(case "the event queue pops the earliest-Instant entry first (pop-min)"
  (doc    "The scheduler's event queue is a time-ordered priority queue keyed by `Instant` (design §4.1).
           Modeled here as a recursive-sum linked list kept ASCENDING by wake Instant — the idiomatic
           Cadenza value-heap structure (a `Q.QCons` of `(Instant, label, rest)`). `q-insert` walks to
           the first entry the new one is `before?` and splices in, so the FRONT is always the minimum.
           Insert A@3s then B@1s; the front label is `B` (the 1-second event), NOT A — pop-min returns
           the earliest event, which is how the clock knows what to fast-forward to next.")
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
            (type Q QNil (QCons (Tuple Instant String Q)))
            (def (q-insert (: q Q) (: t Instant) (: v String))
              (match q
                ((Q.QNil _) (Q.QCons (tuple t v (Q.QNil ()))))
                ((Q.QCons (tuple ht hv rest))
                  (if (before? t ht)
                      (Q.QCons (tuple t v (Q.QCons (tuple ht hv rest))))
                      (Q.QCons (tuple ht hv (q-insert rest t v)))))))
            (def (q-front (: q Q))
              (match q
                ((Q.QNil _) "empty")
                ((Q.QCons (tuple _ hv _)) hv)))
            (def (main)
              (let ((q0 (Q.QNil ()))
                    (q1 (q-insert q0 (Instant.Instant 3000000000) "A"))
                    (q2 (q-insert q1 (Instant.Instant 1000000000) "B")))
                (q-front q2)))
            (export main)))
  (output (: "B" String)))

(case "same-Instant queue entries resume in FIFO insertion order (§3.4 tie-break)"
  (doc    "The FIFO same-time tie-break the corpus determinism rests on (design §3.4, confirmed against
           bach's `push_back`/`pop_front`): two events at the SAME Instant resume in INSERTION order. A
           new entry equal in time to existing ones is spliced AFTER them (the `q-insert` else-branch
           recurses PAST an equal head because `before?` is strict `<`, §3.4 case above). Insert B@1s
           then B2@1s; the front is `B` (inserted first), not B2. A `<=`-based insert would put B2 first
           and silently break every same-time-event ordering in a simulation.")
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
            (type Q QNil (QCons (Tuple Instant String Q)))
            (def (q-insert (: q Q) (: t Instant) (: v String))
              (match q
                ((Q.QNil _) (Q.QCons (tuple t v (Q.QNil ()))))
                ((Q.QCons (tuple ht hv rest))
                  (if (before? t ht)
                      (Q.QCons (tuple t v (Q.QCons (tuple ht hv rest))))
                      (Q.QCons (tuple ht hv (q-insert rest t v)))))))
            (def (q-front (: q Q))
              (match q
                ((Q.QNil _) "empty")
                ((Q.QCons (tuple _ hv _)) hv)))
            (def (main)
              (let ((q0 (Q.QNil ()))
                    (q1 (q-insert q0 (Instant.Instant 1000000000) "B"))
                    (q2 (q-insert q1 (Instant.Instant 1000000000) "B2")))
                (q-front q2)))
            (export main)))
  (output (: "B" String)))

(case "draining the event queue yields entries in time-order with FIFO same-time ties"
  (doc    "The whole-queue witness of the scheduler's event order (design §4.2 example): insert four
           events out of order — A@3s, B@1s, B2@1s, main@5s — then DRAIN front-to-back. The result is
           `B,B2,A,main`: the two 1-second events first in insertion order (FIFO tie-break, §3.4), then
           the 3-second, then the 5-second. This is EXACTLY the event order the §4.2 corpus repro
           (increment 3) expects the live scheduler to produce — here it is the pure-queue proof of that
           ordering, gated today with no continuations, so increment 3 only has to show the scheduler
           drives tasks in this same order.")
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
            (type Q QNil (QCons (Tuple Instant String Q)))
            (def (q-insert (: q Q) (: t Instant) (: v String))
              (match q
                ((Q.QNil _) (Q.QCons (tuple t v (Q.QNil ()))))
                ((Q.QCons (tuple ht hv rest))
                  (if (before? t ht)
                      (Q.QCons (tuple t v (Q.QCons (tuple ht hv rest))))
                      (Q.QCons (tuple ht hv (q-insert rest t v)))))))
            (def (q-drain (: q Q))
              (match q
                ((Q.QNil _) "")
                ((Q.QCons (tuple _ hv rest))
                  (match rest
                    ((Q.QNil _) hv)
                    ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
            (def (main)
              (let ((q0 (Q.QNil ()))
                    (q1 (q-insert q0 (Instant.Instant 3000000000) "A"))
                    (q2 (q-insert q1 (Instant.Instant 1000000000) "B"))
                    (q3 (q-insert q2 (Instant.Instant 1000000000) "B2"))
                    (q4 (q-insert q3 (Instant.Instant 5000000000) "main")))
                (q-drain q4)))
            (export main)))
  (output (: "B,B2,A,main" String)))

(case "the ready-queue is a plain FIFO — spawned-ready tasks run in enqueue order"
  (doc    "Beside the time-ordered event queue, the scheduler keeps a READY queue for work that can run
           at the current instant without a wake-time (a freshly-spawned task's thunk, a resumed
           continuation) — design §4.1's `ready-push`. It is a plain FIFO (no time key): push appends to
           the back, pop takes the front. Enqueue A then B then C; draining yields `A,B,C`. This pins the
           ready-queue order that, together with the event queue's time order, fixes the scheduler's
           deterministic interleave.")
  (input  (do
            (type R RNil (RCons (Tuple String R)))
            (def (r-push (: r R) (: v String))
              (match r
                ((R.RNil _) (R.RCons (tuple v (R.RNil ()))))
                ((R.RCons (tuple hv rest)) (R.RCons (tuple hv (r-push rest v))))))
            (def (r-drain (: r R))
              (match r
                ((R.RNil _) "")
                ((R.RCons (tuple hv rest))
                  (match rest
                    ((R.RNil _) hv)
                    ((R.RCons _) (String.concat hv (String.concat "," (r-drain rest))))))))
            (def (main)
              (let ((r0 (R.RNil ()))
                    (r1 (r-push r0 "A"))
                    (r2 (r-push r1 "B"))
                    (r3 (r-push r2 "C")))
                (r-drain r3)))
            (export main)))
  (output (: "A,B,C" String)))
