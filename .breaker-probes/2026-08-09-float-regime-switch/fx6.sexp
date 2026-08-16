(case "fx6 NaN born in the ARM — s2−s2 is 0.0 while finite and NaN once the thread saturates; canonical equality distinguishes them per dispatch"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: a Float64))
              (handle E a
                ((step () s
                  (let ((s2 (* s s)))
                    (let ((d (- s2 s2)))
                      (resume (if (= d 0.0) 1 (if (= d Float64.nan) 2 0)) s2)))))
                (+ (* 10 (E.step)) (E.step))))
            (export main)))
  (call   main (: 1.0e100 Float64)) (output (: 12 Int64))
  (call   main (: 1.0e50 Float64)) (output (: 11 Int64)))
