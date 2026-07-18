; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DES ⇄ v-effects E5 STEP-3 GATE REPRO — from v-discrete-event-sim (the forcing consumer)
; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; This is the concrete scheduler + repro that step 3 (STORED / ESCAPING `k` — a `Cont` captured in
; one activation and `apply`d from another) must make green. DES increment 4 (`run-sim`) lands the
; moment this runs to the expected value. Both verticals gate this same case.
;
; TODAY (E5 step 1+2 only) every case here DECLINES cleanly with:
;   "this handler is not yet reducible by the tail-resumptive fold (cross-function or non-tail resume
;    arrives in a later increment)"
; — the correct classification: `sleep`'s `k` is stored past its activation, so it is a step-3 case,
; NOT a miscompile. When step 3 lands, the decline flips to the recorded value.
;
; The SCHEDULER API v-effects builds `Cont` against (design §4.1): the sleep arm binds a 4th param `k`
; = the reified rest of the task; the scheduler stores `(wake-instant, k)` in a time-ordered pqueue,
; pops the minimum, sets the clock to its wake instant (the FAST-FORWARD), and `(resume unit …)`s that
; stored k — the resume happening from the SCHEDULER-STEP activation, not the perform site. `now` is a
; tail-resumptive arm (no `k`; reads the clock in place) and already works today.

; ── substrate (DES increment 1, LANDED as spec/semantics/27-discrete-event-simulation.sexp) ─────────
(do
  (type Duration (Duration UInt64))
  (type Instant  (Instant  UInt64))
  (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
  (def (inst-ns (: t Instant))  (match t ((Instant.Instant n) n)))
  (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
  (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
  (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))

  ; ── the Sim effect ────────────────────────────────────────────────────────────────────────────
  (effect Sim
    (op sleep (-> Duration Unit))    ; suspend until now+d; k stored, resumed once from the loop
    (op now   (-> Unit Instant)))    ; read the clock — tail-resumptive, works today

  ; ── a task ────────────────────────────────────────────────────────────────────────────────────
  (def (worker (: label String) (: d Duration))
    (do (Sim.sleep d)               ; ← STEP-3 suspension point: captures k = "return label after wake"
        label))                     ; runs AFTER the stored k is resumed from the scheduler loop

  ; ── the minimal single-task scheduler distillation ──────────────────────────────────────────────
  ; State = the clock (Instant). sleep files the wake instant and resumes k with the clock advanced.
  ; This is the smallest program that stores k across activations; the full §4.2 2-task interleave (a
  ; pqueue of several k's, popped min-first, FIFO tie-break) is the next case once this runs.
  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))                       ; tail-resumptive: clock in place
        (sleep (d) s k                                   ; k = reified rest of the task
          (let ((wake (at s d)))                         ; fast-forward target
            (resume unit wake))) )                       ; resume the stored k with the clock advanced
      (worker "done" (secs 3))))
  (export main))
; EXPECTED once step 3 lands:  (: "done" String)   — the task sleeps 3s, the clock fast-forwards to 3s,
; the stored k resumes and returns "done". A todo→fail flip here = a real miscompile (k not resumed /
; clock not advanced / double-resume).
