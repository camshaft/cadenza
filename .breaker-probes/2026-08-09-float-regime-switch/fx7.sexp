(case "fx7 NEGATIVE ZERO through the state thread — canonical equality separates -0.0 from +0.0, and IEEE addition washes the sign out mid-thread"
  (input  (do
            (effect E (op probe (-> Int64)))
            (def (main (: a Float64))
              (handle E (* -1.0 a)
                ((probe () s
                  (resume (if (= s 0.0) 1 0) (+ s 0.0))))
                (+ (* 10 (E.probe)) (E.probe))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 1 Int64))
  (call   main (: 5.0 Float64)) (output (: 0 Int64)))
