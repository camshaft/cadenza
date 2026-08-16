(case "cr2 RETRY-until-success — a recursive driver re-runs a conditionally-aborting region until the drawn attempt passes, counting tries"
  (input  (do
            (effect C (op draw (-> Int64)))
            (effect R (op fail (-> Int64 Int64)))
            (def (attempt-once)
              (handle R 0
                ((fail (v) u v))
                (let ((a (C.draw)))
                  (if (= (% a 3) 0) (* 1000 a) (do (R.fail -1) 999)))))
            (def (retry (: tries Int64))
              (let ((r (attempt-once)))
                (if (< r 0) (retry (+ tries 1)) (+ (* 100 (+ tries 1)) (/ r 1000)))))
            (def (main (: n Int64))
              (handle C n
                ((draw () s (resume s (+ s 1))))
                (retry 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103 Int64))
  (call   main (: 1 Int64)) (output (: 303 Int64))
  (call   main (: -2 Int64)) (output (: 300 Int64)))
