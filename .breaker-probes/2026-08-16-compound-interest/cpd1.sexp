(case "cpd1 a COMPOUND-INTEREST ladder — grow applies the seed rate percent truncating answering the new principal, skim withdraws answering the remainder, and the two-point rate difference compounds so the gap between the runs WIDENS every grow row while the skims subtract identically"
  (input  (do
            (effect C
              (op grow (-> Int64))
              (op skim (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle C (: 200 Int64)
                ((grow () p
                  (resume (+ p (/ (* p (+ (% n 4) 3)) 100))
                          (+ p (/ (* p (+ (% n 4) 3)) 100))))
                 (skim (v) p (resume (- p v) (- p v))))
                (let ((a (C.grow)))
                  (let ((b (C.grow)))
                    (let ((c (C.skim 30)))
                      (let ((d (C.grow)))
                        (let ((e (C.grow)))
                          (let ((f (C.skim 30)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210220190199208178 Int64))
  (call   main (: 0 Int64)) (output (: 206212182187192162 Int64)))
