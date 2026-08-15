(case "egy1 EGYPTIAN multiplication — each step tests the multiplier's low bit (accumulating the doubled multiplicand when odd) answering odd*100 plus the running product's low digits, acc reads the exact product, and the seeds' bit patterns (all-ones versus alternating) fire the accumulation on different steps"
  (input  (do
            (effect E
              (op step (-> Int64))
              (op acc (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (+ n 5) (: 7 Int64) (: 0 Int64))
                ((step () st
                  (match st
                    ((tuple mult mcand accv)
                      (if (= (% mult 2) 1)
                          (resume (+ 100 (% (+ accv mcand) 100))
                                  (tuple (/ mult 2) (* mcand 2) (+ accv mcand)))
                          (resume (% accv 100)
                                  (tuple (/ mult 2) (* mcand 2) accv))))))
                 (acc () st
                  (match st ((tuple mult mcand accv) (resume accv st)))))
                (let ((a (E.step)))
                  (let ((b (E.step)))
                    (let ((c (E.step)))
                      (let ((d (E.step)))
                        (let ((e (E.acc)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 107121149105105 Int64))
  (call   main (: 0 Int64)) (output (: 107007135035035 Int64)))
