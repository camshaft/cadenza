; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DES ⇄ v-effects E5 STEP-3 GATE — GENUINELY-ESCAPING continuation (deferred resume-thunk, applied
; cross-activation). from v-discrete-event-sim (the forcing consumer). REVISED 2026-07-19 after
; v-effects' SEMANTIC-GAP finding (their note 8911) — the clock-advance is now EXPRESSED IN THE PROGRAM.
; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; PRIOR REVISIONS: v1 bound k but RESUMED IN PLACE → folded via step-2 (didn't force step-3). v2 made k
; escape via `(scheduler-step wake k)` + `(stored-k unit)` — but that had a SEMANTIC BUG v-effects
; correctly caught: `(stored-k unit)` applies the RAW continuation and NEVER threads `wake` into the
; handler state, so a FAITHFUL reduction leaves the clock at the seed 0 and yields 0, not 5e9 — the
; clock-advance lived only in a doc comment, not in the code. Grading it →5e9 would have forced v-effects
; to INVENT a per-effect rule ("sleep's op-arg magically becomes the new clock"), violating "never invent
; effect semantics". CORRECT (v3, this file).
;
; THE FIX (v-effects note 8911 option A — NO invented semantics): the escaping continuation is a DEFERRED
; RESUME-THUNK `(fn (_u) (resume unit wake))`. `resume`'s SECOND ARG `wake` IS the new handler state (the
; advanced clock) — the exact resume-with-state form my COMMITTED corpus already uses IN PLACE (case at
; 27-discrete-event-simulation.sexp:367, `(resume unit (at s d))`). Here that same resume is DEFERRED into
; a thunk that ESCAPES the sleep arm to a SEPARATE top-level fn `scheduler-step`, which APPLIES it cross-
; activation `(resume-thunk unit)`. So: (1) the continuation genuinely ESCAPES (still forces step-3 — the
; resume is not in-place; it leaves the arm and fires from another activation), AND (2) the clock-advance
; is EXPRESSED in the program (the arm threads `wake` via `resume`'s new-state arg — the compiler reads it,
; does not guess it). This also SIMPLIFIES v-effects' reify: the re-installed handler's seed is `resume`'s
; explicit new-state arg (`wake`), NOT a value derived/guessed from the op-arg.

(do
  (type Instant (Instant UInt64))
  (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))

  (effect Sim
    (op sleep (-> Instant Unit))     ; suspend until `wake`; the resume is DEFERRED + fired cross-activation
    (op now   (-> Unit Instant)))    ; read the clock (= handler state) — tail-resumptive, works today

  ; scheduler-step: a SEPARATE activation. Receives the wake instant + the DEFERRED resume-thunk that
  ; escaped the sleep arm, and APPLIES it. (The full run-sim pops the min-wake (wake, resume-thunk) from a
  ; pqueue in the scheduler state; here the single-task distillation hands the one thunk straight through.)
  ; `(resume-thunk unit)` fires the deferred `(resume unit wake)` — the cross-activation resume-with-state.
  (def (scheduler-step (: wake Instant) resume-thunk)
    (resume-thunk unit))

  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))                       ; tail-resumptive: read the clock in place
        (sleep (wake) s                                  ; wake = the op-arg (absolute target Instant)
          (scheduler-step wake                           ; the thunk ESCAPES to scheduler-step, applied there
            (fn (_u) (resume unit wake)))) )             ; DEFERRED resume: new handler-state = wake (the clock advance, IN the program)
      (do (Sim.sleep (Instant.Instant 5000000000))       ; sleep to t=5s (Instant ns; single-op form)
          (inst-ns (Sim.now)))))                         ; observe the fast-forwarded clock after resume
  (export main))
; EXPECTED once the escaping-resume-thunk capability (v-effects FACE-1 + wake-seeded reify) lands:
;   (: 5000000000 Int64) — scheduler-step fires the deferred resume with new state = wake (5e9); the
;   continuation `(inst-ns (Sim.now))` reads the advanced clock (5s). FAITHFUL: the arm threaded `wake` via
;   `resume`, so no invented semantics. A todo→fail flip = a miscompile (thunk not applied / new-state not
;   threaded / resumed with the original seed 0 → 0 / double-resume / dropped continuation).
; NOTE: single-task genuinely-escaping-continuation core. The multi-task woken-ORDER grade ("B,A,main")
; needs the full pqueue + trace threading = the increment-4 run-sim value-graded corpus case; this gate
; isolates the one thing the escaping capability adds over step-2: a resume that leaves its arm (deferred
; in a thunk) and fires from another activation, carrying the advanced state.
