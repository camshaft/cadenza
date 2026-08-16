(case "cr3 a BOUNDED retry budget — two attempts then give up, the fallback negating the last failed attempt"
  (input  (do
            (effect C (op draw (-> Int64)))
            (effect R (op fail (-> Int64 Int64)))
            (def (attempt-once)
              (handle R 0
                ((fail (v) u v))
                (let ((a (C.draw)))
                  (if (= (% a 5) 0) (* 1000 a) (do (R.fail (- 0 a)) 999)))))
            (def (retry (: tries Int64))
              (if (>= tries 2)
                  -999999
                  (let ((r (attempt-once)))
                    (if (> r -999999)
                        (if (>= r 0) (+ (* 100 (+ tries 1)) (/ r 1000)) 
                            (if (>= (+ tries 1) 2) r (retry (+ tries 1))))
                        r))))
            (def (main (: n Int64))
              (handle C n
                ((draw () s (resume s (+ s 1))))
                (retry 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64))
  (call   main (: 4 Int64)) (output (: 205 Int64))
  (call   main (: 1 Int64)) (output (: -2 Int64)))
