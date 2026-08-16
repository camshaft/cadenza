(case "CSE does not hoist a repeated DIV whose guard makes it dead — no spurious trap at d=0"
  (doc    "Trap-freedom under the O2 global-CSE Core pass (99827365b): `(100 / d)` appears TWICE in the
           else-branch of `(if (= d 0) 999 …)`, so a common-subexpression eliminator is tempted to
           compute it ONCE — but hoisting it above the guard would evaluate `100/d` at d=0 and TRAP,
           defeating the guard that exists to prevent exactly that. d=5 → both uses read 20 → 220;
           d=0 → the guard returns 999 and the div is NEVER evaluated (a hoisting CSE traps here
           instead). Opt-sweep O0..O3 must all yield 999 at d=0 — the CSE tier (O2) cannot change the
           observable by moving a partial (trapping) expression out of its guarding branch. The
           trap-safety companion of the select-ification guard pins, on the fresh CSE pass.")
  (input  (do
            (def (main (: d Int64))
              (if (= d 0)
                999
                (+ (* (/ 100 d) 10) (/ 100 d))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 220 Int64))
  (call   main (: 0 Int64)) (output (: 999 Int64)))
