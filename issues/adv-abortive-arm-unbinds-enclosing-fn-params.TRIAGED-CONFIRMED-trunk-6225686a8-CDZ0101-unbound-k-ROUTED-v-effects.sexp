; FINDING (breaker, 2026-07-28): a handle with an ABORTIVE arm (an arm path that returns
; WITHOUT resume) breaks the enclosing fn's PARAM visibility for the whole handle form, and
; with a recursive body leaks a MANGLED INTERNAL NAME in the diagnostic. Both backends.
;
;   F1 (false unbound, both backends): ANY reference to the enclosing fn's param — in the arm
;      guard OR even in the SEED — rejects "unbound name k" as soon as one arm path is abortive:
;        (handle Sim (tuple 0 k) ((step (u) st (if ... -999 (resume ...)))) body)  -> unbound k@SEED
;      The IDENTICAL program with the arm made all-resume computes (3). So the abortive-arm
;      lowering re-homes the handle into a context that lost main's params.
;   F2 (diagnostic leak): abortive arm + RECURSIVE body:
;        (def (loop ...) (... (Sim.step))) under the abortive handle
;      -> error [CDZ0201]: `loop#eff2` has no body — a compiler-internal effect-specialization
;      mangling (loop#eff2) surfaces in a user-facing message. Whatever the supportedness ruling
;      (abortive-under-recursion may be a legit not-yet), the message must name `loop`, not the
;      synthesized specialization, and should say not-yet-reducible per the :5120 discipline.
;
; Deadline/timeout sims (the DES idiom motivating this) can't be written until F1 fixes.
;
; GRADED REPRO (F1; expected = abortive cut at the seed-threaded limit):
(case "an abortive arm reads the enclosing fn's param through the handler seed"
  (input  (do
        (effect Sim (op step (-> Unit Int64)))
        (def (main (: k Int64))
          (handle Sim (tuple 0 k)
            ((step (u) st (if (>= (. st 0) (. st 1)) -999 (resume (. st 0) (tuple (+ (. st 0) 1) (. st 1))))))
            (+ (Sim.step) (+ (Sim.step) (Sim.step)))))
        (export main)))
  (call   main (: 2 Int64)) (output (: -999 Int64))
  (call   main (: 10 Int64)) (output (: 3 Int64)))
