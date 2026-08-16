(case "an unannotated module member called at TWO sites infers consistently"
  (doc    "No-over-restriction sentinel for the expected_arrow_for_lambda re-entry guard (9171a02be,
           the arrow_lambdas_in_progress set that fixed the ~1024-deep compile-hang / browser
           worker-stack P0): a module member with UNANNOTATED params called at TWO call sites must
           infer once and serve both — 32 + 212 = 244. A guard that returned None too eagerly would
           under-infer the second site (the exact regression risk of a re-entry set); 11-modules:98
           pins the repro's NAME-RESOLUTION half, this pins the inference half.")
  (input  (do
            (module Temp (def (c-to-f c) (+ (/ (* c 9) 5) 32)))
            (def (main) (+ (Temp.c-to-f 0) (Temp.c-to-f 100)))
            (export main)))
  (output (: 244 Int64)))

(case "an unannotated member calling ANOTHER unannotated member infers through nested recovery"
  (doc    "The nested-recovery sentinel: `f` (unannotated) calls `Temp.g` (also unannotated) — TWO
           lambda-param recoveries active at once, which the arrow_lambdas_in_progress re-entry set
           must allow (they are DIFFERENT lambdas; only re-entering the SAME one mid-recovery cycles).
           g(10)+1 = 21. An over-broad guard keyed on 'any recovery in progress' would recover None
           for g and under-infer the nest — the companion sentinel to the twice-called face.")
  (input  (do
            (module Temp
              (def (g y) (* y 2))
              (def (f x) (+ (Temp.g x) 1)))
            (def (main) (Temp.f 10))
            (export main)))
  (output (: 21 Int64)))
