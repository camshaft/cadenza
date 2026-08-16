(case "nz1 NEGATIVE ZERO as op argument — canonical equality distinguishes -0.0 from +0.0 at the boundary, the sign probe splits three ways"
  (input  (do
            (effect F (op sign (-> Float64 Int64)))
            (def (main (: a Float64))
              (handle F 0
                ((sign (x) s
                  (resume (if (= x 0.0)
                              (if (> (/ 1.0 x) 0.0) 1 2)
                              3)
                          s)))
                (+ (* 10 (F.sign (* a 1.0))) (F.sign (* a -1.0)))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 13 Int64))
  (call   main (: 2.5 Float64)) (output (: 33 Int64)))
