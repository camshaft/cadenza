(case "fo2 float-sum ORDER as keys: negative zero vs positive zero payloads are DISTINCT keys"
  (input  (do
            (type Reading (Temp Float64) (Missing))
            (def (main (: x Float64))
              (let ((s (Set.of (list (Temp (- x x)) (Temp (* (- x x) -1.0)) (Missing)))))
                (Set.len s)))
            (export main)))
  (call   main (: 2.5 Float64)) (output (: 3 Int64)))
