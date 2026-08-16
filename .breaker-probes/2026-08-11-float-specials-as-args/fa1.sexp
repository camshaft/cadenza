(case "fa1 NaN and infinity born in the BODY cross as op ARGUMENTS — canonical equality in the arm identifies NaN, self-equality holds for all"
  (input  (do
            (effect F (op probe (-> Float64 Int64)))
            (def (main (: a Float64))
              (handle F 0
                ((probe (x) s (resume (if (= x x) (if (= x Float64.nan) 3 1) 2) s)))
                (let ((s2 (* a a)))
                  (let ((nan (- s2 s2)))
                    (+ (* 100 (F.probe nan)) (+ (* 10 (F.probe s2)) (F.probe a)))))))
            (export main)))
  (call   main (: 1.0e200 Float64)) (output (: 311 Int64))
  (call   main (: 2.5 Float64)) (output (: 111 Int64)))
