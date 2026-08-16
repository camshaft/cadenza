(case "negative-literal match arms dispatch a runtime scrutinee including Int64.min"
  (doc    "Negative literals as match-arm PROBES, including the boundary the range machinery must
           carry exactly: arms -1, Int64.min, 0, wildcard over a runtime scrutinee — computed -1
           (via (- 0 k)) hits the -1 arm (1000s), the LITERAL Int64.min scrutinee hits its own arm
           (100s — a probe stored as an unsigned/wrapped constant, or a range analysis clamping min,
           misroutes to the wildcard), 0 and a positive take their slots → 1234. The corpus matches
           negative SCRUTINEES against positive probes and guards min in division; negative-literal
           ARMS (and min AS a probe) were unpinned in the scalar dispatch engine.")
  (input  (do
            (def (cls (: n Int64))
              (match n
                (-1 1)
                (-9223372036854775808 2)
                (0 3)
                (_ 4)))
            (def (main (: k Int64))
              (+ (* 1000 (cls (- 0 k)))
                 (+ (* 100 (cls -9223372036854775808))
                    (+ (* 10 (cls 0)) (cls 99)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1234 Int64)))
