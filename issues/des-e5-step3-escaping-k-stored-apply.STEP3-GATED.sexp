; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DES ⇄ v-effects E5 STEP-3 GATE — GENUINELY-ESCAPING-k (stored, applied cross-activation)
; from v-discrete-event-sim (the forcing consumer). REVISED 2026-07-18 after v-effects' critical finding.
; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; WHY THIS REVISION: the PRIOR version bound k but RESUMED IN PLACE ((resume unit (at s d))) with k
; otherwise unused, so it folded tail-resumptively (E5 step 2 already handles that) and ran to
; 5000000000 WITHOUT forcing step 3 — the store/pop/apply was only in COMMENTS, not code. v-effects
; correctly flagged that grading it green does not prove the step-3 heap machinery. This version makes k
; GENUINELY ESCAPE: the sleep arm hands k to a SEPARATE `scheduler-step` function that APPLIES it via
; ordinary application `(k unit)` — NOT `resume` (which is arm-only sugar; using it outside an arm is
; CDZ0201). A continuation that left its arm and is invoked from another activation is exactly what the
; real step 3 (Ty::Cont heap rep + defunctionalized frame chain) must realize — so this repro is a
; genuine forcing consumer, declining cleanly today and flipping to a value only when step 3 lands.
;
; THE SHAPE (the increment-4 run-sim distillation): the scheduler's `sleep` arm does NOT resume k; it
; passes (wake, k) to `scheduler-step`, a SEPARATE top-level function. scheduler-step sets the clock to
; the wake instant (the FAST-FORWARD) and APPLIES the stored continuation `(k unit)` — resuming the task
; from a different activation than the one that captured it. That cross-activation apply is step 3.
;
; Value grade once step 3 lands: the task sleeps to 5s, scheduler-step fast-forwards the clock to the
; wake instant and resumes the stored k, which reads (now) and observes 5s → (: 5000000000 Int64). A
; nonzero result proves the STORED k was applied AND the clock advanced across the cross-activation
; resume; 0 would mean k was resumed with the pre-sleep clock. TODAY: declines cleanly ("not yet
; reducible by the tail-resumptive fold") — NONE of this folds in place because k escapes to another fn.

(do
  (type Instant (Instant UInt64))
  (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))

  (effect Sim
    (op sleep (-> Instant Unit))     ; suspend until `wake`; k is STORED + resumed cross-activation
    (op now   (-> Unit Instant)))    ; read the clock — tail-resumptive, works today

  ; scheduler-step: a SEPARATE activation. Receives the wake instant + the STORED continuation k that
  ; escaped the sleep arm, and APPLIES it. (The full run-sim pops the min-wake (wake, k) from a pqueue in
  ; the scheduler state; here the single-task distillation hands the one stored k straight through.) The
  ; `(stored-k unit)` is the ordinary-application form of apply(k, v) — the cross-activation resume.
  (def (scheduler-step (: wake Instant) stored-k)
    (stored-k unit))

  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))                  ; tail-resumptive: read the clock in place
        (sleep (wake) s k                           ; k = reified rest of the task
          (scheduler-step wake k)) )                ; k ESCAPES to scheduler-step, applied there
      (do (Sim.sleep (Instant.Instant 5000000000))  ; sleep to t=5s (Instant ns; single-op form)
          (inst-ns (Sim.now)))))                    ; observe the fast-forwarded clock after resume
  (export main))
; EXPECTED once real step 3 (Ty::Cont heap rep + stored-k applied cross-activation) lands:
;   (: 5000000000 Int64) — scheduler-step applies the stored k with the clock advanced to the wake
;   instant, and (now) reads 5s. A todo→fail flip = a miscompile (k not applied / clock not advanced /
;   double-resume / dropped continuation).
; NOTE: this is the SINGLE-task genuinely-escaping-k core. The multi-task woken-ORDER grade ("B,A,main")
; needs the full pqueue + trace threading = the increment-4 run-sim value-graded corpus case; this gate
; isolates the one thing step 3 must add over step 2: a k that escapes its arm and resumes elsewhere.
