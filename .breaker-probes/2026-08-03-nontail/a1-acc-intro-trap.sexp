(case "a1 accumulator-intro must not hoist the body's trapping op ahead of the zero-iteration exit"
  (input  (do
            (def (sum-div (: n Int64) (: d Int64))
              (if (= n 0) 0 (+ (/ 100 d) (sum-div (- n 1) d))))
            (def (main (: n Int64) (: d Int64))
              (sum-div n d))
            (export main)))
  (call   main (: 0 Int64) (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 3 Int64) (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 3 Int64) (: 0 Int64)) (trap "divide by zero"))
