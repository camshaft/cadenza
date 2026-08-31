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
(case
  "a Duration constructor `secs` scales a UInt64 count to nanoseconds"
  (doc
    "`(secs 5)` builds a `Duration` of 5_000_000_000 ns — the bach `5.s()` DurationLiteral
           (ext.rs:10), scaled by 1e9. `Duration` is a nominal newtype over `UInt64` (§3.2), so the
           returned value prints as `(: 5000000000 Duration)` — the ns count with the nominal name. This
           is the base unit-scaling the whole clock rests on: a task never handles a bare `UInt64`, only
           `secs`/`ms`/`us`/`ns`, so the Duration discipline holds by construction.")
  (input
    (do
      (type Duration (Duration UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (main) (secs 5))
      (export main)))
  (output (: 5000000000 Duration)))

(case
  "the DES Duration constructors ms / us / ns each scale to the right nanosecond magnitude"
  (doc
    "`ms`/`us`/`ns` mirror bach's `100.ms()` / `.us()` / `.ns()` (ext.rs:10): `(ms 100)` =
           100_000_000 ns, `(us 100)` = 100_000 ns, `(ns 100)` = 100 ns. Pins each constructor's scale
           factor (1e6 / 1e3 / 1) so a future edit can't silently transpose two of them — a wrong scale
           would make every sleep in the wrong unit compile clean yet run wrong. Runs each via a boundary
           `(call …)` arg (a runtime UInt64 that cannot fold) so the multiply executes as a real
           instruction, then unwraps to the ns count.")
  (input
    (do
      (type Duration (Duration UInt64))
      (def (ms (: n UInt64)) (Duration.Duration (* n 1000000)))
      (def (us (: n UInt64)) (Duration.Duration (* n 1000)))
      (def (ns (: n UInt64)) (Duration.Duration n))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration v) v)))
      (def
        (main (: kind UInt64) (: n UInt64))
        (if (= kind 0) (dur-ns (ms n)) (if (= kind 1) (dur-ns (us n)) (dur-ns (ns n)))))
      (export main)))
  (call main (: 0 UInt64) (: 100 UInt64))
  (output (: 100000000 UInt64))
  (call main (: 1 UInt64) (: 100 UInt64))
  (output (: 100000 UInt64))
  (call main (: 2 UInt64) (: 100 UInt64))
  (output (: 100 UInt64)))

(case
  "`at` advances an Instant by a Duration (wake-time computation)"
  (doc
    "`(at t d)` = `t + d` — the scheduler's wake-time computation (design §3.2, §4.1: the sleep
           arm files a continuation at `(at (clock-of s) d)`). `Instant`/`Duration` are distinct nominal
           newtypes over `UInt64`; `at` unwraps both, adds the ns, and re-wraps as an `Instant`. From
           `t0 = 0` and a 3-second span, the wake Instant is 3_000_000_000 ns. This is the only way a
           point advances by a span, so it pins the point-plus-span arithmetic the event queue keys on.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (def (main) (at (Instant.Instant 0) (secs 3)))
      (export main)))
  (output (: 3000000000 Instant)))

(case
  "`since` is the span between two Instants (later minus earlier)"
  (doc
    "`(since later earlier)` = `later − earlier`, a `Duration` — bach's `Instant::elapsed`
           (time.rs). It is the dual of `at`: `at` adds a span to a point, `since` subtracts two points
           to a span. `(since t3 t0)` where t3 is 3 s and t0 is 0 yields a 3-second `Duration`
           (3_000_000_000 ns). This is what `sleep-until` is derived from — `(sleep-until t)` =
           `(sleep (since (now) t))` (§4) — so the span-from-now-to-a-target computation is pinned.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def
        (at (: t Instant) (: d Duration))
        (Instant.Instant (+ (inst-ns t) (match d ((Duration.Duration n) n)))))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def
        (since (: later Instant) (: earlier Instant))
        (Duration.Duration (- (inst-ns later) (inst-ns earlier))))
      (def
        (main)
        (let ((t0 (Instant.Instant 0)) (t3 (at (Instant.Instant 0) (secs 3)))) (since t3 t0)))
      (export main)))
  (output (: 3000000000 Duration)))

(case
  "the span between two equal Instants is zero — the inclusive lower boundary of `since`"
  (doc
    "The zero-span boundary, the exact edge BELOW the `since`-underflow trap. `(since t t)` =
           `t − t` = 0: two equal Instants yield the SMALLEST valid `Duration` (a zero span), NOT a trap.
           This pins that the span domain is inclusive at 0 — `since` traps only when the second (earlier)
           argument is strictly LATER (the underflow case pinned above), and a same-instant span is a
           legitimate zero, not an off-by-one into the trap. Load-bearing for the scheduler: a task that
           computes `sleep(since (now) (now))` or two events filed at the same instant produce a zero-delay
           span (which files at the current clock, §3.4 tie-break), not a spurious trap or a wrap. Graded
           via a runtime `(call …)` arg so the subtract is a real instruction rather than a const fold.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration v) v)))
      (def
        (since (: later Instant) (: earlier Instant))
        (Duration.Duration (- (inst-ns later) (inst-ns earlier))))
      (def (main (: a UInt64)) (dur-ns (since (Instant.Instant a) (Instant.Instant a))))
      (export main)))
  (call main (: 3000000000 UInt64))
  (output (: 0 UInt64)))

(case
  "`before?` orders two Instants (the event-queue comparison)"
  (doc
    "`(before? a b)` = the underlying `UInt64` `<` — the ONLY comparison the time-ordered event
           queue uses to sort wake-times (design §3.2, §4.1). `1 ns` is before `3 ns` (true); the strict
           `<` means an Instant is NOT before itself (the same-time case is a tie-break, §3.4, not a
           before?-true — pinned in the next case). Determinism of the whole sim rests on this being a
           total strict order over the ns counter.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (def (main) (before? (Instant.Instant 1) (Instant.Instant 3)))
      (export main)))
  (output (: true Bool)))

(case
  "`before?` is a STRICT order — an Instant is not before an equal Instant (same-time is a tie-break)"
  (doc
    "The same-time boundary: `(before? t t)` is FALSE for equal Instants. This is load-bearing for
           the FIFO same-time tie-break (§3.4) — two events at the SAME instant are NOT ordered by
           `before?`; they resume in INSERTION order, which the queue realizes by inserting a new
           equal-key entry AFTER the existing equal-key ones (the `q-insert` cases below). A `<=` here
           instead of `<` would break FIFO by making a later same-time insert compare 'before' an earlier
           one. Both same-time Instants are 1 s.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (def (main) (before? (Instant.Instant 1000000000) (Instant.Instant 1000000000)))
      (export main)))
  (output (: false Bool)))

(case
  "an Instant and a Duration are DISTINCT nominal types — a point cannot be used where a span is expected"
  (doc
    "The strong-typing the operator asked for (§3.2, verbatim 'strong typing'): `Instant` and
           `Duration` both erase to `UInt64`, but they are DISTINCT nominal newtypes — a point vs a span.
           `at` expects `(: d Duration)` as its second argument; passing an `Instant` there is rejected
           CDZ0203 naming both nominal types, even though both erase to `UInt64`. This pins the
           point/span type safety: you cannot accidentally add two Instants or sleep for an Instant. The
           program's outcome is the rejection.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (def (main) (at (Instant.Instant 0) (Instant.Instant 3)))
      (export main)))
  (error CDZ0203))

(case
  "a runtime UInt64 underflow inside a newtype constructor argument still traps"
  (doc
    "The trap-preservation face of the nominal-newtype erasure: `(D.D (- k 5))` wraps a CHECKED
           UInt64 subtraction in a Duration-style constructor. The wrap must not launder the check —
           at k = 3 the underflow traps 'integer overflow' exactly as the bare `(- k 5)` does; at
           k = 12 the wrapped 7 destructures back out. Pins that erasure (a newtype IS its rep at run
           time) erases the TYPE, not the inner operation's overflow discipline — the ctor-argument
           companion of the scale-constructor pins above (whose arithmetic never underflows).")
  (input
    (do (type D (D UInt64)) (def (main (: k UInt64)) (match (D.D (- k 5)) ((D v) v))) (export main)))
  (call main (: 3 UInt64))
  (trap "integer overflow")
  (call main (: 12 UInt64))
  (output (: 7 UInt64)))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 1 — the time-ordered event queue (priority queue, FIFO same-time tie-break, §3.4, §4.1)
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "the event queue pops the earliest-Instant entry first (pop-min)"
  (doc
    "The scheduler's event queue is a time-ordered priority queue keyed by `Instant` (design §4.1).
           Modeled here as a recursive-sum linked list kept ASCENDING by wake Instant — the idiomatic
           Cadenza value-heap structure (a `Q.QCons` of `(Instant, label, rest)`). `q-insert` walks to
           the first entry the new one is `before?` and splices in, so the FRONT is always the minimum.
           Insert A@3s then B@1s; the front label is `B` (the 1-second event), NOT A — pop-min returns
           the earliest event, which is how the clock knows what to fast-forward to next.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def (q-front (: q Q)) (match q ((Q.QNil _) "empty") ((Q.QCons #tuple(_ hv _)) hv)))
      (def
        (main)
        (let
          ((q0 (Q.QNil ()))
            (q1 (q-insert q0 (Instant.Instant 3000000000) "A"))
            (q2 (q-insert q1 (Instant.Instant 1000000000) "B")))
          (q-front q2)))
      (export main)))
  (output (: "B" String))
  (live-objects known-leak))

(case
  "same-Instant queue entries resume in FIFO insertion order (§3.4 tie-break)"
  (doc
    "The FIFO same-time tie-break the corpus determinism rests on (design §3.4, confirmed against
           bach's `push_back`/`pop_front`): two events at the SAME Instant resume in INSERTION order. A
           new entry equal in time to existing ones is spliced AFTER them (the `q-insert` else-branch
           recurses PAST an equal head because `before?` is strict `<`, §3.4 case above). Insert B@1s
           then B2@1s; the front is `B` (inserted first), not B2. A `<=`-based insert would put B2 first
           and silently break every same-time-event ordering in a simulation.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def (q-front (: q Q)) (match q ((Q.QNil _) "empty") ((Q.QCons #tuple(_ hv _)) hv)))
      (def
        (main)
        (let
          ((q0 (Q.QNil ()))
            (q1 (q-insert q0 (Instant.Instant 1000000000) "B"))
            (q2 (q-insert q1 (Instant.Instant 1000000000) "B2")))
          (q-front q2)))
      (export main)))
  (output (: "B" String))
  (live-objects known-leak))

(case
  "draining the event queue yields entries in time-order with FIFO same-time ties"
  (doc
    "The whole-queue witness of the scheduler's event order (design §4.2 example): insert four
           events out of order — A@3s, B@1s, B2@1s, main@5s — then DRAIN front-to-back. The result is
           `B,B2,A,main`: the two 1-second events first in insertion order (FIFO tie-break, §3.4), then
           the 3-second, then the 5-second. This is EXACTLY the event order the §4.2 corpus repro
           (increment 3) expects the live scheduler to produce — here it is the pure-queue proof of that
           ordering, gated today with no continuations, so increment 3 only has to show the scheduler
           drives tasks in this same order.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (q-drain (: q Q))
        (match
          q
          ((Q.QNil _) "")
          ((Q.QCons #tuple(_ hv rest))
            (match
              rest
              ((Q.QNil _) hv)
              ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
      (def
        (main)
        (let
          ((q0 (Q.QNil ()))
            (q1 (q-insert q0 (Instant.Instant 3000000000) "A"))
            (q2 (q-insert q1 (Instant.Instant 1000000000) "B"))
            (q3 (q-insert q2 (Instant.Instant 1000000000) "B2"))
            (q4 (q-insert q3 (Instant.Instant 5000000000) "main")))
          (q-drain q4)))
      (export main)))
  (output (: "B,B2,A,main" String))
  (live-objects known-leak))

(case
  "INTERLEAVED pops and inserts keep the event queue min-ordered across live mutation"
  (doc
    "The LIVE-MUTATION face (the draining case above inserts everything THEN drains): pops and
           inserts interleave as in a running scheduler where wakes fire while events drain — pop A,
           insert B EARLIER than the resident D (a later insert beating an already-resident entry),
           pop B, insert C mid-queue; mode 2 adds Z@0 straight to the FRONT after two pops have
           restructured the spine. Trace A,B,C,D / A,B,Z,C,D.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (q-pop (: q Q))
        (match
          q
          ((Q.QNil _) #tuple("empty" (Q.QNil ())))
          ((Q.QCons #tuple(_t hv rest)) #tuple(hv rest))))
      (def
        (q-drain (: q Q))
        (match
          q
          ((Q.QNil _) "")
          ((Q.QCons #tuple(_ hv rest))
            (match
              rest
              ((Q.QNil _) hv)
              ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
      (def
        (main (: mode Int64))
        (do
          (def q1 (q-insert (q-insert (Q.QNil ()) (Instant.Instant 3) "A") (Instant.Instant 4) "D"))
          (def p1 (q-pop q1))
          (match
            p1
            (#tuple(v1 q2)
              (do
                (def q3 (q-insert q2 (Instant.Instant 1) "B"))
                (def p2 (q-pop q3))
                (match
                  p2
                  (#tuple(v2 q4)
                    (do
                      (def q5 (q-insert q4 (Instant.Instant 2) "C"))
                      (def q6 (if (= mode 2) (q-insert q5 (Instant.Instant 0) "Z") q5))
                      (String.concat
                        v1
                        (String.concat "," (String.concat v2 (String.concat "," (q-drain q6)))))))))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: "A,B,C,D" String))
  (call main (: 2 Int64))
  (output (: "A,B,Z,C,D" String))
  (live-objects known-leak))

(case
  "the ready-queue is a plain FIFO — spawned-ready tasks run in enqueue order"
  (doc
    "Beside the time-ordered event queue, the scheduler keeps a READY queue for work that can run
           at the current instant without a wake-time (a freshly-spawned task's thunk, a resumed
           continuation) — design §4.1's `ready-push`. It is a plain FIFO (no time key): push appends to
           the back, pop takes the front. Enqueue A then B then C; draining yields `A,B,C`. This pins the
           ready-queue order that, together with the event queue's time order, fixes the scheduler's
           deterministic interleave.")
  (input
    (do
      (type R RNil (RCons (Tuple String R)))
      (def
        (r-push (: r R) (: v String))
        (match
          r
          ((R.RNil _) (R.RCons #tuple(v (R.RNil ()))))
          ((R.RCons #tuple(hv rest)) (R.RCons #tuple(hv (r-push rest v))))))
      (def
        (r-drain (: r R))
        (match
          r
          ((R.RNil _) "")
          ((R.RCons #tuple(hv rest))
            (match
              rest
              ((R.RNil _) hv)
              ((R.RCons _) (String.concat hv (String.concat "," (r-drain rest))))))))
      (def
        (main)
        (let
          ((r0 (R.RNil ())) (r1 (r-push r0 "A")) (r2 (r-push r1 "B")) (r3 (r-push r2 "C")))
          (r-drain r3)))
      (export main)))
  (output (: "A,B,C" String))
  (live-objects known-leak))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 2 — the `Sim` effect declaration + task API shape (§4). `now` is tail-resumptive and
; works TODAY; `sleep` is E5-general and declines cleanly until v-effects' E5 step 3 (increment 4).
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "Sim.now reads the scheduler clock — a tail-resumptive handler arm that works today"
  (doc
    "The `Sim` effect's `now` op (design §4, §3.3) reads the current simulated time. Its handler arm
           is TAIL-RESUMPTIVE — `(now (u) s (resume s s))` binds no continuation `k` and resumes the clock
           `s` in place — so it needs no E5 general continuation and runs on the current compiler. The
           scheduler state IS the clock (an `Instant`); seeding the handle at `(Instant.Instant 42)` and
           reading `(now)` yields 42 ns. This pins that the read-the-clock op is available for tasks from
           increment 2, independent of the `sleep`/`spawn` suspension machinery (which awaits step 3).")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (effect Sim (op now (-> Unit Instant)))
      (def (now) (Sim.now))
      (def (main) (handle Sim (Instant.Instant 42) ((now (u) s (resume s s))) (inst-ns (now))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a task-facing `sleep` wrapper performs Sim.sleep — the surface a simulation is written against"
  (doc
    "The task-facing `(sleep d)` wrapper (design §4: `(def (sleep d) (Sim.sleep d))`) is the surface
           an operator writes a simulation against — straight-line effectful code where each suspending op
           is a `perform`. Here a one-task program sleeps for a Duration then returns a label; the scheduler
           handler's `sleep` arm binds the continuation `k` (the reified rest of the task) and would file
           it at the wake instant `(at s d)`, fast-forward the clock, and resume it. This is the E5 step-3
           case (a STORED/escaping `k` resumed from a different activation), so TODAY it declines cleanly
           ('not yet reducible by the tail-resumptive fold') and scores `todo`; when v-effects' E5 step 3
           lands it flips to `pass` producing (: \"done\" String) — the increment-4 landing signal. A
           `todo`→`fail` flip here is a real miscompile (k not resumed / clock not advanced / double-resume).")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def (sleep (: d Duration)) (Sim.sleep d))
      (def (worker (: label String) (: d Duration)) (do (sleep d) label))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s k (resume unit (at s d))))
          (worker "done" (secs 3))))
      (export main)))
  (output (: "done" String)))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 3 — THE E5 STEP-3 GATE REPRO (the shared contract with v-effects). §4.2 example distilled:
; a task sleeps, the clock fast-forwards to its wake instant, the STORED continuation resumes and the
; task observes the advanced clock. Scores `todo` until E5 step 3; a `todo`→`fail` flip is a miscompile.
; (Full 2-task interleave with a pqueue + FIFO tie-break follows once this single-task case is green.)
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "a task that sleeps then reads `now` observes the fast-forwarded clock (E5 step-3 gate)"
  (doc
    "The end-to-end fast-forward proof (design §4.2, §5 'fast-forward = zero real time'): a task
           `(do (sleep (secs 3)) (now))` suspends at the sleep, the scheduler files its continuation at
           wake = `(at clock (secs 3))` = 3_000_000_000 ns, sets the clock to that wake instant (the
           FAST-FORWARD), and resumes the stored continuation — which then reads `now` and observes the
           ADVANCED clock (3 s), not the original 0. The recorded value is 3_000_000_000. This is the
           single-task distillation of the §4.2 repro and the exact case v-effects gates their E5 step 3
           against (a `Cont` captured at the perform site and applied from the scheduler-step activation,
           with the handler state advanced across the resume). TODAY it declines cleanly and scores `todo`;
           it flips to `pass` when step 3 lands (DES increment 4). The clock-advances-across-resume aspect
           is the load-bearing bit — a step-3 implementation that resumed with the ORIGINAL state `s`
           instead of the advanced `(at s d)` would return 0 here, so this case also pins that the arm's
           threaded next-state (not the pre-perform state) reaches the resumed continuation.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s k (resume unit (at s d))))
          (do (Sim.sleep (secs 3)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 3000000000 Int64)))

(case
  "a genuinely-escaping continuation (deferred resume-thunk) resumes cross-activation at the advanced clock"
  (doc
    "The step-3 core (design §4.2), distilled to the ONE thing an ESCAPING continuation adds over the
           step-2 tail-resumptive fold: a resume that LEAVES its arm and fires from another activation,
           carrying the advanced clock. The `sleep` arm does NOT resume in place; it hands a DEFERRED
           resume-thunk `(fn (_u) (resume unit wake))` to a SEPARATE top-level `scheduler-step`, which
           applies it cross-activation `(resume-thunk unit)`. `resume`'s second arg `wake` IS the new
           handler state (the advanced clock) — the same resume-with-new-state the tail-resumptive cases
           use, but DEFERRED into an escaping thunk. So the clock-advance is expressed IN THE PROGRAM (no
           per-effect op-arg-as-state-setter magic): the reified continuation re-enters the handler
           re-seeded with `wake`, and `(Sim.now)` reads the advanced clock → 5_000_000_000. This is the
           contract v-effects' E5 step-3 (deferred-resume-thunk refold via do-aware leading-hole) realizes;
           it declined cleanly until step-3 landed. A `pass`→`fail` flip here is a real miscompile (thunk
           not applied / resumed with the original seed 0 → 0 / double-resume / dropped continuation). This
           single-task escaping core is what the increment-4 multi-task run-sim pqueue composes on.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def (scheduler-step (: wake Instant) resume-thunk) (resume-thunk unit))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep (wake) s (scheduler-step wake (fn (_u) (resume unit wake)))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a stored continuation is popped from a time-ordered pqueue (tuple-payload match) and resumed"
  (doc
    "The multi-task step over the single-escaping-continuation case: the scheduler's pending
           resumptions live in a time-ordered priority queue whose entries are `(waketime, boxed-k, rest)`
           — a single tuple-payload ctor destructured into 3 binders (`(PQCons (Tuple Instant KBox PQ))`
           arrives as ONE payload, the same single-tuple-payload distinction as fold_ctor_match slice-2,
           not N separate payload args). Popping the earliest event match-binds all three fields, then
           extracts and applies the boxed continuation. This exercises the pqueue POP that a multi-task
           run-sim is built on: `(match q ((PQCons (tuple wake kb rest)) (match kb ((KBox k) (k unit)))))`.
           The continuation is a deferred resume-thunk closing over its wake instant (`(fn (_u) (resume
           unit wake))`), extracted THROUGH the multi-binder tuple pattern and applied — so `(now)` reads
           the advanced clock → 5_000_000_000. This declined until v-inference extended `fold_ctor_match`
           to the multi-payload/tuple-payload path (each tuple binder substituted to its `Elem(i)`
           projection) — the reach beyond the single-payload single-binder extract; it regression-locks
           that a continuation stored in a pqueue entry and popped via a multi-field match still folds.
           A `pass`→`fail` flip is a miscompile (continuation not applied / clock not advanced / resumed
           at the pre-pop state).")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def
        (pop-apply (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (pop-apply
                (PQ.PQCons #tuple(wake (KBox.KBox (fn (_u) (resume unit wake))) (PQ.PQNil ()))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; KNOWN LIMITATION (parked, concierge ruling 2026-07-19) — the multi-task run-sim cap.
; ────────────────────────────────────────────────────────────────────────────────────────────────
; The cases above deliver the SINGLE-task fast-forward scheduler end to end: a task's `sleep`
; continuation is reified as an escaping deferred-resume-thunk, stored in a time-ordered pqueue entry,
; popped via a (multi-payload) match, and resumed cross-activation at the advanced clock (→ 5e9). That
; is the full E5-step-3 escaping-continuation machinery, delivered and regression-locked.
;
; The MULTI-task run-sim (several tasks whose continuations coexist in the pqueue, resumed in time order)
; additionally requires a continuation to be routed through a DATA-RECURSIVE sorted-insert (`pins`) — the
; priority queue's own time-ordering insert — before it is popped and resumed. That shape DECLINES to fold
; today (a clean feature-absent decline, NOT a miscompile): the deferred-resume fold's recursion-unfold
; re-resolves the rebuilt arm and drops the pin on the spliced resume-closure's captured handler-arm
; binder (`wake`), poisoning the closure before the pop-fold sees it. Root-caused + co-confirmed with
; v-inference; PARKED pending a re-resolve DESIGN pass (graft the unfolded node under the handler arm so
; lexical re-resolution reaches `wake`), a v-effects + v-inference co-design — checkpointed at v-effects
; WIP `f4d45a53e`. Resumes on a forcing consumer or operator priority. Until then the multi-task scheduler
; would have to bound its inserts (hand-unrolled, capped task count) rather than use a data-recursive pins,
; so the general unbounded multi-task run-sim is intentionally NOT yet in this corpus — the cap is recorded
; here rather than hidden behind a passing-but-bounded case.
; ────────────────────────────────────────────────────────────────────────────────────────────────
; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 1 — event-queue determinism edge cases (design §8: guard the §3.4 FIFO tie-break at N=3,
; interleaved times, and the sleep(0)/pop-then-reinsert scheduler-step primitive). Pure substrate.
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "three events at the same Instant drain in FIFO insertion order (tie-break at N=3)"
  (doc
    "The §3.4 FIFO same-time tie-break must hold beyond the N=2 case already pinned: THREE events
           all at 1 s, inserted A then B then C, drain `A,B,C` — insertion order preserved because
           `q-insert` splices each equal-key entry AFTER the existing ones (strict `<` in `before?`). A
           `<=`-based insert would reverse them. Determinism at N≥3 is what makes a multi-event same-tick
           simulation reproducible.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (q-drain (: q Q))
        (match
          q
          ((Q.QNil _) "")
          ((Q.QCons #tuple(_ hv rest))
            (match
              rest
              ((Q.QNil _) hv)
              ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
      (def (I (: n UInt64)) (Instant.Instant n))
      (def
        (main)
        (let
          ((q0 (Q.QNil ()))
            (q1 (q-insert q0 (I 1000000000) "A"))
            (q2 (q-insert q1 (I 1000000000) "B"))
            (q3 (q-insert q2 (I 1000000000) "C")))
          (q-drain q3)))
      (export main)))
  (output (: "A,B,C" String))
  (live-objects known-leak))

(case
  "interleaved same/different Instants keep FIFO within a time and ascending across times"
  (doc
    "The combined ordering invariant: insert X@2s, P@1s, Y@2s, Q@1s (interleaving two times, two
           events each) and the drain is `P,Q,X,Y` — the 1-second pair first in insertion order, then the
           2-second pair in insertion order. Pins that the queue is BOTH globally time-ascending AND
           per-instant FIFO simultaneously (not just one or the other) — the exact ordering the scheduler
           fast-forward loop relies on to pick the next event deterministically.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (q-drain (: q Q))
        (match
          q
          ((Q.QNil _) "")
          ((Q.QCons #tuple(_ hv rest))
            (match
              rest
              ((Q.QNil _) hv)
              ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
      (def (I (: n UInt64)) (Instant.Instant n))
      (def
        (main)
        (let
          ((q0 (Q.QNil ()))
            (q1 (q-insert q0 (I 2000000000) "X"))
            (q2 (q-insert q1 (I 1000000000) "P"))
            (q3 (q-insert q2 (I 2000000000) "Y"))
            (q4 (q-insert q3 (I 1000000000) "Q")))
          (q-drain q4)))
      (export main)))
  (output (: "P,Q,X,Y" String))
  (live-objects known-leak))

(case
  "a RUNTIME-count generated batch insorts fully time-ordered, verified by a sortedness walk"
  (doc
    "The parametric-count companion of the fixed-eight drain below: `fill` insorts n events whose
           times are GENERATED (`(i·7) mod 13` — a scrambled, colliding sequence the author never wrote
           out), and `is-sorted?` walks the result answering whether every adjacent pair is `<=` (true at
           n=10, false on the first inversion). The PROPERTY-style witness: the ordering invariant holds
           for a batch whose size and contents arrive at run time, not only for hand-laid events. The
           sortedness answer is a `Bool` — the property itself — not a count-or-magic-int (idiomatic
           strong typing: a checked yes/no is a Bool, never an in-band sentinel integer).")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (def
        (insort (: t Instant) (: q (List Instant)))
        (match
          q
          (#list() #list(t))
          (#list(h (.. rest))
            (if (before t h) (List.concat #list(t) q) (List.concat #list(h) (insort t rest))))))
      (def
        (fill (: i Int64) (: n Int64) (: q (List Instant)))
        (if (>= i n) q (fill (+ i 1) n (insort (Instant.Instant (UInt64.wrap (% (* i 7) 13))) q))))
      (def
        (is-sorted? (: q (List Instant)))
        (match
          q
          (#list() true)
          (#list(a (.. rest))
            (match
              rest
              (#list() true)
              (#list(b (.. more)) (if (<= (inst-ns a) (inst-ns b)) (is-sorted? rest) false))))))
      (def (main (: n Int64)) (is-sorted? (fill 0 n #list())))
      (export main)))
  (call main (: 10 Int64))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "a deep out-of-order queue drains fully time-sorted with FIFO across every tie-group"
  (doc
    "A larger stress of the ordering invariant: eight events at times 5,2,8,2,1,8,3,2 (labels a..h)
           inserted in that order drain `e,b,d,h,g,a,c,f` — ascending by time (1,2,2,2,3,5,8,8) with FIFO
           preserved WITHIN each of the three tie-groups (b,d,h at 2s; c,f at 8s). Confirms the recursive
           insert keeps a correct total order at realistic depth, not just at 2-3 entries.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (q-drain (: q Q))
        (match
          q
          ((Q.QNil _) "")
          ((Q.QCons #tuple(_ hv rest))
            (match
              rest
              ((Q.QNil _) hv)
              ((Q.QCons _) (String.concat hv (String.concat "," (q-drain rest))))))))
      (def (I (: n UInt64)) (Instant.Instant n))
      (def
        (main)
        (let
          ((q
              (q-insert
                (q-insert
                  (q-insert
                    (q-insert
                      (q-insert
                        (q-insert (q-insert (q-insert (Q.QNil ()) (I 5) "a") (I 2) "b") (I 8) "c")
                        (I 2)
                        "d")
                      (I 1)
                      "e")
                    (I 8)
                    "f")
                  (I 3)
                  "g")
                (I 2)
                "h")))
          (q-drain q)))
      (export main)))
  (output (: "e,b,d,h,g,a,c,f" String))
  (live-objects known-leak))

(case
  "a zero-Duration sleep files at the current instant — pop-min then reinsert keeps order"
  (doc
    "The `sleep(0)` / scheduler-step primitive edge: a zero-`Duration` wake files at the CURRENT
           clock (`(at clock (ns 0))` = clock), so it sorts before any positive-delay event. Insert A@1s
           and B@0ns: B is the front (pop-min). Pop B (drop the front), then a zero-sleep from the
           fast-forwarded clock reinserts C at 0 — still before A@1s. Result `B|C`: B popped first, C now
           the front. Pins that zero durations and the pop-then-reinsert step (the scheduler loop's core
           op) do not misorder the queue.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (ns (: n UInt64)) (Duration.Duration n))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type Q QNil (QCons (Tuple Instant String Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v String))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def (q-pop (: q Q)) (match q ((Q.QNil _) (Q.QNil ())) ((Q.QCons #tuple(_ _ rest)) rest)))
      (def (q-front-label (: q Q)) (match q ((Q.QNil _) "empty") ((Q.QCons #tuple(_ hv _)) hv)))
      (def
        (q-front-inst (: q Q))
        (match q ((Q.QNil _) (Instant.Instant 0)) ((Q.QCons #tuple(ht _ _)) ht)))
      (def
        (main)
        (let
          ((clock (Instant.Instant 0))
            (q0 (Q.QNil ()))
            (q1 (q-insert q0 (at clock (ns 1000000000)) "A"))
            (q2 (q-insert q1 (at clock (ns 0)) "B"))
            (front (q-front-label q2))
            (q3 (q-pop q2))
            (q4 (q-insert q3 (at (q-front-inst q2) (ns 0)) "C")))
          (String.concat front (String.concat "|" (q-front-label q4)))))
      (export main)))
  (output (: "B|C" String))
  (live-objects known-leak))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 4 (partial) — the PRIMARY/SECONDARY termination decision (§7.4, §7.5; operator-required).
; Pure logic (no continuations): the scheduler-step consults this each loop to decide Running/Done/
; Deadlock. Sim ends when the PRIMARY count hits zero (parked background tasks are discarded), NOT when
; the queue drains; deadlock = live primaries with no ready work and an empty timer queue.
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "the scheduler terminates on zero primaries and detects deadlock when live primaries have no work"
  (doc
    "The §7.4 termination rule the operator required (§7.5: distinguish background tasks from ones
           the sim cares about completing): `sched-status(ready-nonempty?, timers-nonempty?, primaries)`
           returns Done when the primary count is 0 — the sim ends even if background continuations are
           still parked (bach's `.primary()` Guard hitting zero, `task.rs:85`) — Running when live
           primaries still have ready work or pending timers, and Deadlock when live primaries have
           NEITHER (zero ready work AND an empty timer queue with a nonzero primary count — a detectable
           error state, §7.4). Graded across all five branches: 0=Running, 1=Done, 2=Deadlock. Pins that
           termination is PRIMARY-driven (not queue-drain), so a server-loop background task cannot hold
           the sim open and a genuine deadlock is distinguished from normal completion.")
  (input
    (do
      (type Status Running Done Deadlock)
      (def
        (sched-status (: ready-nonempty Bool) (: timers-nonempty Bool) (: primaries UInt64))
        (if
          (= primaries 0)
          (Status.Done)
          (if (or ready-nonempty timers-nonempty) (Status.Running) (Status.Deadlock))))
      (def
        (show-status (: st Status))
        (match st ((Status.Running _) 0) ((Status.Done _) 1) ((Status.Deadlock _) 2)))
      (def (main (: rn Bool) (: tn Bool) (: p UInt64)) (show-status (sched-status rn tn p)))
      (export main)))
  (call main (: true Bool) (: false Bool) (: 2 UInt64))
  (output (: 0 UInt64))
  (call main (: false Bool) (: true Bool) (: 1 UInt64))
  (output (: 0 UInt64))
  (call main (: false Bool) (: false Bool) (: 0 UInt64))
  (output (: 1 UInt64))
  (call main (: true Bool) (: true Bool) (: 0 UInt64))
  (output (: 1 UInt64))
  (call main (: false Bool) (: false Bool) (: 3 UInt64))
  (output (: 2 UInt64)))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 4 (single-task) — the `run-sim` scheduler + `sleep-until`. The single-task sleep/fast-
; forward path RUNS today (E5 step-2 folds a tail-resumptive sleep arm); the multi-task escaping-k
; interleave awaits E5 step 3. These cases pin the working single-task scheduler + the derived API.
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "a run-sim scheduler fast-forwards the clock through two sequential sleeps in one task"
  (doc
    "The `run-sim` surface (design §4): a `handle Sim` whose state is the clock, the `sleep` arm
           advancing it (`(at s d)`) and resuming, `now` reading it tail-resumptively. A task that sleeps
           2 s then 3 s in sequence threads the clock through BOTH suspensions — the second sleep starts
           from the fast-forwarded 2 s and lands at 5 s — so `(now)` reads 5_000_000_000 ns. Pins the
           single-task fast-forward scheduler end-to-end (a task written as straight-line effectful code,
           the clock jumping event-to-event), which runs TODAY on the E5 step-2 tail-resumptive fold — the
           multi-task interleave (several stored continuations popped from a pqueue) is what still awaits
           E5 step 3. This is the concrete `sleep`-does-not-block-a-wall-clock guarantee (§5): a 5-second
           simulation costs two queue steps, not five seconds.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def
        (run-sim thunk)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s k (resume unit (at s d))))
          (thunk)))
      (def (task) (do (Sim.sleep (secs 2)) (Sim.sleep (secs 3)) (inst-ns (Sim.now))))
      (def (main) (run-sim task))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "`sleep-until` sleeps to an absolute Instant — the span is target minus now (since t now)"
  (doc
    "The derived `sleep-until` (design §4): sleep until an ABSOLUTE `Instant`, not for a relative
           span. `(sleep-until t)` = `(sleep (since t (now)))` — the span from now TO t, which is `t − now`
           because `since later earlier = later − earlier` (§3.2), so the future target `t` is the `later`
           operand. ARG ORDER IS LOAD-BEARING: `(since (now) t)` would compute `now − t`, a UInt64
           UNDERFLOW for any future target (`t > now`) — a compile-provable overflow the compiler rejects
           (CDZ0304), NOT a wrong value. This case pins the correct order: a task at t=0 sleeps-until 4 s
           (clock jumps to 4 s) then sleeps-until 7 s (jumps to 7 s), so `(now)` reads 7_000_000_000 ns.
           A regression flipping the operands would trap at compile time, caught here as a todo/fail.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (def
        (since (: later Instant) (: earlier Instant))
        (Duration.Duration (- (inst-ns later) (inst-ns earlier))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def (now) (Sim.now))
      (def (sleep-until (: t Instant)) (Sim.sleep (since t (now))))
      (def
        (run-sim thunk)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s k (resume unit (at s d))))
          (thunk)))
      (def
        (task)
        (do
          (sleep-until (Instant.Instant 4000000000))
          (sleep-until (Instant.Instant 7000000000))
          (inst-ns (now))))
      (def (main) (run-sim task))
      (export main)))
  (output (: 7000000000 Int64)))

; ────────────────────────────────────────────────────────────────────────────────────────────────
; Increment 1 — clock-boundary safety: `at` traps on nanosecond-clock overflow rather than wrapping.
; ────────────────────────────────────────────────────────────────────────────────────────────────
(case
  "advancing the clock past the UInt64 nanosecond range traps rather than silently wrapping"
  (doc
    "The clock-boundary safety property: `at` (wake-time computation) is `clock + duration` over
           `UInt64` ns, so a span that carries the clock past `UInt64.max` (~584 years of ns) TRAPS
           'integer overflow' — it does NOT silently wrap to a small `Instant`. This is load-bearing for
           a DES: a silent wrap would place a far-future event at a tiny timestamp, catastrophically
           misordering the event queue (the wrapped event would pop before earlier real events). Trapping
           makes an over-long simulation a clean failure, not a silent miscompute. Graded via runtime
           `(call …)` args so the add is a real instruction: `at(18446744073709551610, 10)` overflows
           (2^64-6 + 10 > 2^64-1) and traps; the control `at(1s, 2s)` = 3s returns normally. Pins that the
           unsigned clock arithmetic keeps its overflow guard (matching the numeric-model UInt64 semantics).")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (def
        (main (: t UInt64) (: d UInt64))
        (inst-ns (at (Instant.Instant t) (Duration.Duration d))))
      (export main)))
  (call main (: 18446744073709551610 UInt64) (: 10 UInt64))
  (trap "integer overflow")
  (call main (: 1000000000 UInt64) (: 2000000000 UInt64))
  (output (: 3000000000 UInt64)))

(case
  "computing a span with the Instants reversed (earlier minus later) traps rather than wrapping"
  (doc
    "The span-boundary safety property, dual to the clock-overflow case above and pinning the
           `sleep-until` arg-order discipline. `(since later earlier)` = `later − earlier` over `UInt64`
           ns (§3.2), so calling it with the arguments REVERSED — `(since earlier later)` where the
           second Instant is the LATER one — is `earlier − later` < 0, a `UInt64` UNDERFLOW that TRAPS
           'integer overflow'; it does NOT silently wrap to a near-`UInt64.max` `Duration`. This is
           load-bearing: `sleep-until` is `(sleep (since t (now)))` = `t − now` for a future target `t`,
           and the WRONG order `(since (now) t)` = `now − t` underflows for any `t > now`. A silent wrap
           would hand the scheduler an astronomically-large bogus span (~584 years), filing the event so
           far in the future it never fires (or, post-`at`, wrapping the wake-time to misorder the queue).
           Trapping turns an arg-order bug into a clean failure instead of a silent misschedule. Graded
           via runtime `(call …)` so the subtract is a real instruction: reversed `since(1s, 5s)` = 1s−5s
           underflows and traps; correct-order `since(5s, 1s)` = 4s returns normally.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def
        (since (: later Instant) (: earlier Instant))
        (Duration.Duration (- (inst-ns later) (inst-ns earlier))))
      (def
        (main (: later UInt64) (: earlier UInt64))
        (dur-ns (since (Instant.Instant later) (Instant.Instant earlier))))
      (export main)))
  (call main (: 1000000000 UInt64) (: 5000000000 UInt64))
  (trap "integer overflow")
  (call main (: 5000000000 UInt64) (: 1000000000 UInt64))
  (output (: 4000000000 UInt64)))

(case
  "a Duration constructor scaling a count past the UInt64 nanosecond range traps rather than wrapping"
  (doc
    "The constructor-boundary safety property, completing the time-arithmetic overflow family with
           the clock-overflow (`at`) and span-underflow (`since`) cases. `secs` scales its `UInt64` count
           by 1e9 (§3.2), so a count large enough that `n * 1_000_000_000` exceeds `UInt64.max` (~1.8e10
           seconds ≈ 584 years) OVERFLOWS the multiply and TRAPS 'integer overflow' — it does NOT silently
           wrap to a tiny `Duration`. This is load-bearing for the same reason as the clock/span cases: a
           silent wrap would hand the scheduler a `Duration` far smaller than intended, firing a
           long-delay event almost immediately and misordering the event queue. Trapping turns an
           over-long duration into a clean failure. Graded via a runtime `(call …)` arg so the multiply is
           a real instruction: `secs(18446744074)` (just past `UInt64.max / 1e9`) traps; the control
           `secs(5)` = 5_000_000_000 returns normally.")
  (input
    (do
      (type Duration (Duration UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration v) v)))
      (def (main (: n UInt64)) (dur-ns (secs n)))
      (export main)))
  (call main (: 18446744074 UInt64))
  (trap "integer overflow")
  (call main (: 5 UInt64))
  (output (: 5000000000 UInt64)))

(case
  "a run-sim task's suspension count is DATA-DRIVEN by a list of sleep durations"
  (doc
    "The :755 fast-forward pin threads the clock through two STATICALLY-WRITTEN sleeps; here the
           sleeps come from a LIST walked recursively — `sleep-all [2, k]` — so the NUMBER of
           suspensions the clock threads through is decided by data, and each perform sits under a
           recursive call frame (the handler must re-enter across recursion, not just straight-line
           code). Final now/1e9 = 5 at k=3, 2 at k=0 (a zero-duration sleep still suspends and
           resumes, contributing nothing). A tail-resumptive fold that only handled lexically-visible
           performs (or lost the clock across the recursive frames) computes the k=3 row wrong.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def
        (sleep-all (: ds (List Int64)))
        (match
          ds
          (#list() unit)
          (#list(h (.. t)) (do (Sim.sleep (secs (UInt64.wrap h))) (sleep-all t)))))
      (def
        (main (: k Int64))
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s (resume unit (at s d))))
          (do
            (sleep-all (List.push (List.push #list() 2) k))
            (Int64.wrap (/ (inst-ns (Sim.now)) (: 1000000000 UInt64))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a rope label rides a pqueue entry through insort to its time-ordered slot"
  (doc
    "The event-queue pins carry scalar/continuation payloads; this entry's SECOND field is a
           runtime ROPE (`(String.concat \"a\" \"x\")`) riding the insort — the heap label must
           travel WITH its time through the recursive insert's prepend spine and read back from the
           sorted position: k=5 sorts first (\"axbc\" → 41), k=25 middles (\"baxc\" → 42), k=99
           lasts (\"bcax\" → 43). A tuple-copy during insort that dropped or re-leafed the rope (or
           paired labels with the wrong times through the prepend recursion) changes the concat.")
  (input
    (do
      (def
        (insort (: q (List (Tuple Int64 String))) (: e (Tuple Int64 String)))
        (match
          q
          (#list() #list(e))
          (#list(h (.. t))
            (if (<= (. e 0) (. h 0)) (List.prepend q e) (List.prepend (insort t e) h)))))
      (def
        (labels (: q (List (Tuple Int64 String))) (: acc String))
        (match q (#list() acc) (#list(h (.. t)) (labels t (String.concat acc (. h 1))))))
      (def
        (main (: k Int64))
        (do
          (def
            q
            (insort
              (insort (insort #list() #tuple(30 "c")) #tuple(k (String.concat "a" "x")))
              #tuple(20 "b")))
          (def s (labels q ""))
          (+
            (* 10 ((. String byte-len) s))
            (if (= s "axbc") 1 (if (= s "baxc") 2 (if (= s "bcax") 3 0))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 41 Int64))
  (call main (: 25 Int64))
  (output (: 42 Int64))
  (call main (: 99 Int64))
  (output (: 43 Int64))
  (live-objects known-leak))

; --- The adjacent-pair timeline window. ---
(case
  "a pairwise MAX-GAP walk over an Instant timeline subtracts adjacent newtype payloads"
  (doc
    "The DES analyses are forward-accumulate; this is the ADJACENT-PAIR window (timeline analysis): List.at i AND i+1 in one frame, subtract newtype payloads at UInt64, Int64.wrap the gap, max-fold. Max gap is the MIDDLE pair (200) — off-by-one windowing or first/last-only walks diverge.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def
        (gap-max (: ts (List Instant)) (: i Int64) (: best Int64))
        (match
          (List.at ts (+ i 1))
          ((Option.Some nxt)
            (do
              (def cur (Option.expect (List.at ts i) "cur"))
              (def gap (Int64.wrap (- (inst-ns nxt) (inst-ns cur))))
              (gap-max ts (+ i 1) (if (> gap best) gap best))))
          ((Option.None _u) best)))
      (def
        (main (: k Int64))
        (gap-max
          #list((Instant.Instant 100)
            (Instant.Instant (UInt64.wrap (+ 150 k)))
            (Instant.Instant 400)
            (Instant.Instant 450))
          0
          0))
      (export main)))
  (call main (: 50 Int64))
  (output (: 200 Int64))
  (live-objects known-leak))

; --- Record event payloads through the pqueue insort. ---
(case
  "record event payloads ride pqueue insort and read both fields from the head"
  (doc
    "The record-payload face of the event queue (scalar/rope/continuation payloads are
           pinned): each entry pairs a time with a TWO-FIELD record, the runtime k decides which
           entry heads the queue, and BOTH fields read from the popped head's record (19 at k=5
           — the k-entry wins with id 1/pri 9; 31 at k=99 — the 30-entry wins with id 3/pri 1).
           An insort that copied the record payload shallowly against the tuple spine (or swapped
           payloads between entries during the prepend recursion) crosses the id/pri pairs.")
  (input
    (do
      (def
        (insort
          (: q (List (Tuple Int64 (Record (: id Int64) (: pri Int64)))))
          (: e (Tuple Int64 (Record (: id Int64) (: pri Int64)))))
        (match
          q
          (#list() #list(e))
          (#list(h (.. t))
            (if (<= (. e 0) (. h 0)) (List.prepend q e) (List.prepend (insort t e) h)))))
      (def
        (main (: k Int64))
        (do
          (def
            q
            (insort
              (insort #list() #tuple(30 #record((= id 3) (= pri 1))))
              #tuple(k #record((= id 1) (= pri 9)))))
          (match q (#list(h (.. _t)) (+ (* 10 (. (. h 1) id)) (. (. h 1) pri))) (_ -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 19 Int64))
  (call main (: 99 Int64))
  (output (: 31 Int64))
  (live-objects known-leak))

; ── breaker batch 582: the DES event-queue census (a sum-spine priority queue driven through
; insert-rebuild mutation — the file pins ORDER/VALUE but not census). deq1 = build a 10-entry
; queue by sorted insert (each insert rebuilds the traversed prefix), then drain; the value is
; exact (sum 1..10 = 55) and the discarded spine prefixes leak SUPERLINEARLY (32 at n=5, 91 at
; n=10 — ~O(n²/2) undropped QCons+tuple nodes, the insert-rebuild face of the sum-spine leak).
; Flips with the two-shell / tuple-payload reclaim arc.
(case
  "deq1 a DES priority queue built by sorted insert then drained is value-exact and leaks its rebuilt spine prefixes (superlinear)"
  (input
    (do
      (type Instant (Instant UInt64))
      (def
        (before? (: a Instant) (: b Instant))
        (match a ((Instant.Instant x) (match b ((Instant.Instant y) (< x y))))))
      (type Q QNil (QCons (Tuple Instant Int64 Q)))
      (def
        (q-insert (: q Q) (: t Instant) (: v Int64))
        (match
          q
          ((Q.QNil _) (Q.QCons #tuple(t v (Q.QNil ()))))
          ((Q.QCons #tuple(ht hv rest))
            (if
              (before? t ht)
              (Q.QCons #tuple(t v (Q.QCons #tuple(ht hv rest))))
              (Q.QCons #tuple(ht hv (q-insert rest t v)))))))
      (def
        (drain (: q Q) (: acc Int64))
        (match q ((Q.QNil _) acc) ((Q.QCons #tuple(_ hv rest)) (drain rest (+ acc hv)))))
      (def
        (build (: i Int64) (: q Q))
        (if (= i 0) q (build (- i 1) (q-insert q (Instant.Instant (UInt64.wrap (% (* i 7) 11))) i))))
      (def (main (: n Int64)) (drain (build n (Q.QNil ())) 0))
      (export main)))
  (call main (: 10 Int64))
  (output (: 55 Int64))
  (live-objects known-leak))
