; FINDING (breaker, 2026-07-21): a DO-SEQUENCED perform of an OUTER-handled effect, executed
; INSIDE an inner (different-effect) handle, loses its state advance — ALL THREE backends,
; O0..O3 (a shared-lowering bug, not an emit divergence):
;
;   (handle A 0 ((bump …resume 0 (+ s 1)) (get …resume s s))
;     (handle B 100 ((noop …))
;       (do (A.bump unit) (A.get unit))))          → 0   [expected 1]  ← BUG
;
;   same but NO inner handle:                       → 1   (control: do-sequencing fine)
;   same but arith-sequenced (+ (A.bump) (A.get)):  → 2-of-2 correct  (non-do crossing fine)
;   via a helper fn called in the inner body:       → 0   [expected 2] (same bug through a call)
;   interleaved A/B performs (arith + do mix):      → 110 [expected 112] (A's advances lost)
;
; The differentiator is the DO sequencing: a discarded-value perform of the OUTER effect under
; an INNER handle resumes with the STALE state (the bump's (+ s 1) next-state never reaches the
; following operation). Cross-effect data flow ((Dst.put (Src.get))), two-effects-in-one-walk
; (via let-bound perform results), and branch-perform state threading are all pinned and pass —
; those shapes consume every perform's VALUE. Only the do-discarded perform crossing a handler
; level drops its state advance.
;
; Severity: the accumulate-by-do idiom ((do (Diag.emit …) (Diag.emit …) (Diag.collect))) is
; pinned and works at ONE handler depth — any program composing that idiom under one more
; handler silently reads stale state. Wrong VALUE, no error, every backend.

(case "REPRO a do-sequenced outer perform under an inner handle threads its state advance"
  (input  (do
            (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op noop (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 100
                  ((noop (u) t (resume t t)))
                  (do (A.bump unit) (A.get unit)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "CONTROL the same do-sequence at one handler depth threads correctly"
  (input  (do
            (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (do (A.bump unit) (A.get unit))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "CONTROL the arith-sequenced crossing threads correctly"
  (input  (do
            (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op noop (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 100
                  ((noop (u) t (resume t t)))
                  (+ (* 0 (A.bump unit)) (A.get unit)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
