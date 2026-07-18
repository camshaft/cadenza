; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DES ⇄ v-effects E5 STEP-3 GATE REPRO #2 — the MULTI-TASK interleave (GENUINELY ESCAPING k)
; from v-discrete-event-sim (the forcing consumer). Requested by v-effects 2026-07-18:
;   "file the 2-task interleave repro and I will build the real step-3 heap rep against it (Ty::Cont
;    heap rep + frame chain) — that is also what unblocks the closure-capture tier-1 silent-value bug."
; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DISTINCTION FROM GATE #1 (des-e5-step3-scheduler-repro): gate #1 is a SINGLE task whose sleep
; continuation folds tail-resumptively IN PLACE — v-effects served it with two bounded fixes (MR
; ce67417de) and it runs to (: "done" String) WITHOUT the full step-3 heap machinery. THIS repro is
; different: it stores SEVERAL k's in a priority queue and applies each from a DIFFERENT activation (a
; separate `scheduler-step` function pops the min-wake k and resumes it). That is a genuinely ESCAPING
; k — a `Cont` heap value stored in a collection and resumed later — which per the E5 design (§2, §3)
; needs the real `Ty::Cont` heap rep + defunctionalized frame chain. So this is the TRUE step-3 gate.
;
; THE §4.2 SIMULATION (design DESIGN-discrete-event-simulation.md §4.2), value-graded on event ORDER:
;   spawn worker A (sleeps 3s), spawn worker B (sleeps 1s), main sleeps 5s. Virtual clock fast-forwards.
;   Deterministic event order: B wakes @1s, A wakes @3s, main done @5s. Final sim time = 5s.
; We grade on a woken-order trace string built as each task resumes: expected "B,A,main".
;
; THE ESCAPING-k SHAPE v-effects builds Ty::Cont against (E5 design §2-3, apply(k,v) = ordinary
; application of a stored Cont heap value):
;   - sleep arm binds k : Cont Unit Ans; stores (wake-instant, k) in a time-ordered pqueue keyed by
;     Instant (FIFO same-time tie-break, design §3.4); returns to the scheduler loop WITHOUT resuming k.
;   - spawn arm binds k (the spawner's continuation) AND enqueues the child thunk as a fresh task; both
;     the spawner-k and the child become ready.
;   - now is tail-resumptive (reads the clock in place; works today).
;   - scheduler-step (a SEPARATE function — the different activation) pops the earliest (wake, k) from
;     the pqueue, sets clock := wake (the FAST-FORWARD), and applies that STORED k. This cross-activation
;     apply of a stored Cont is the crux of step 3.
;
; TODAY (before real step 3) this declines cleanly ("not yet reducible by the tail-resumptive fold");
; a todo→fail flip is a real miscompile (wrong event order / double-resume / clock not fast-forwarding).
; EXPECTED once step 3 lands: (: "B,A,main" String).
;
; NOTE on the model below: the scheduler state carries the clock + the pqueue of stored k's + a woken
; trace. `Cont Unit Ans` is v-effects' type; here `k` is whatever the handler arm binds. The pqueue is
; the recursive-sum time-ordered structure from the LANDED substrate (spec/semantics/27-*.sexp). This
; is written to the surface v-effects landed for the k-binding arm; only the stored-then-applied-later
; step is what step 3 must realize.

(do
  ; ── substrate (DES increment 1, LANDED) ─────────────────────────────────────────────────────────
  (type Duration (Duration UInt64))
  (type Instant  (Instant  UInt64))
  (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
  (def (inst-ns (: t Instant))  (match t ((Instant.Instant n) n)))
  (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
  (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
  (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))

  ; ── the Sim effect (design §4) ──────────────────────────────────────────────────────────────────
  (effect Sim
    (op sleep (-> Duration Unit))            ; suspend until now+d; k STORED in pqueue, resumed from loop
    (op spawn (-> (-> Unit Unit) Unit))      ; enqueue a task thunk (fn (_u) …); v1 fire-and-forget shape
    (op now   (-> Unit Instant)))            ; read the clock — tail-resumptive

  ; ── the two workers + main (the §4.2 program) ───────────────────────────────────────────────────
  ; each worker sleeps then appends its label to the shared trace via the handler state; here we model
  ; the trace as the handler's answer so grading is on the returned order string.
  ; a worker sleeps then completes. It returns Unit (the spawn thunk type is (-> Unit Unit) in v1 —
  ; fire-and-forget). Its observable is that it RAN (advancing/reading the clock); the woken-ORDER trace
  ; is threaded through the scheduler state by the full run-sim (increment 4), so THIS gate — whose only
  ; job is to exercise escaping-k + fast-forward and type-check cleanly — grades on the FINAL CLOCK, which
  ; is nonzero iff every stored continuation was resumed and the clock fast-forwarded to the last event.
  (def (worker (: d Duration))
    (do (Sim.sleep d) unit))

  ; ── the scheduler: state = the clock (Instant). The FULL run-sim (inc 4) also carries the pqueue of
  ; stored k's + a woken trace; here the sleep arm files (at s d, k) and returns to the loop WITHOUT
  ; resuming k, and a separate scheduler-step pops the min-wake k and APPLIES it (the escaping-k crux).
  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))
        (sleep (d) s k
          ; store (at s d, k) in the pqueue carried in state; return to loop WITHOUT resuming k now.
          ; the scheduler-step (different activation) later pops + applies this stored k.
          (resume unit (at s d)))
        (spawn (t) s k
          ; ready the child thunk and the spawner-k; both runnable at the current clock
          (resume unit s)) )
      (do (Sim.spawn (fn (_u) (worker (secs 3))))
          (Sim.spawn (fn (_u) (worker (secs 1))))
          (Sim.sleep (secs 5))
          (inst-ns (Sim.now)))))
  (export main))
; EXPECTED once real step 3 (Ty::Cont heap rep + stored-k applied cross-activation) lands:
;   both workers wake (B@1s, A@3s), main resumes @5s, and (now) reads the fast-forwarded clock = 5s →
;   (: 5000000000 Int64). A nonzero final clock proves every stored k was resumed and the clock
;   fast-forwarded to the last event; 0 would mean main's sleep-k was resumed with the un-advanced state.
;   (The woken-ORDER trace "B,A,main" is graded by the full run-sim value-graded corpus case in increment
;   4, which threads the trace through scheduler state; this gate isolates the escaping-k mechanism.)
; A todo→fail flip = a miscompile (clock not fast-forwarding / dropped k / double-resume).
