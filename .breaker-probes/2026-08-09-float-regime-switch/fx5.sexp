(case "fx5 the float state SATURATES to infinity mid-thread — a squaring ladder crosses Float64.max, the arm's finite/inf verdict flips per dispatch"
  (input  (do
            (effect E (op sq (-> Int64)))
            (def (main (: a Float64))
              (handle E a
                ((sq () s
                  (let ((s2 (* s s)))
                    (resume (if (> s2 1.7e308) 1 0) s2))))
                (+ (* 10 (E.sq)) (E.sq))))
            (export main)))
  (call   main (: 1.0e100 Float64)) (output (: 1 Int64))
  (call   main (: 1.0e50 Float64)) (output (: 0 Int64))
  (call   main (: 1.0e200 Float64)) (output (: 11 Int64)))
