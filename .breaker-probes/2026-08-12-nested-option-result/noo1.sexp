(case "noo1 an op whose result is NESTED Option (Option Int64) — the arm classifies the state into None / Some None / Some (Some s), the body's nested match distinguishes all three in one run"
  (input  (do
            (effect S (op draw (-> (Option (Option Int64)))))
            (def (main (: n Int64))
              (handle S n
                ((draw () s
                  (resume (if (< s 0)
                              (: (None unit) (Option (Option Int64)))
                              (if (= s 0)
                                  (Some (: (None unit) (Option Int64)))
                                  (Some (Some s))))
                          (- s 1))))
                (let ((f (fn ((: o (Option (Option Int64))))
                           (match o
                             ((Some inner) (match inner ((Some x) (* x 10)) ((None _u) 0)))
                             ((None _u) -5)))))
                  (let ((a (f (S.draw))))
                    (let ((b (f (S.draw))))
                      (let ((c (f (S.draw))))
                        (+ (* 10000 (+ a 9)) (+ (* 100 (+ b 9)) (+ c 9)))))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 190904 Int64))
  (call   main (: 2 Int64)) (output (: 291909 Int64)))
