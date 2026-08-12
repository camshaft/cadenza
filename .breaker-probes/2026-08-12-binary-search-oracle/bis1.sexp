(case "bis1 BINARY SEARCH against an ORACLE effect — the arm only answers -1/0/1 vs the hidden target, the body's recursive driver narrows (lo,hi) from the verdicts, dispatch count is data-dependent"
  (input  (do
            (effect O (op probe (-> Int64 Int64)))
            (def (search (: lo Int64) (: hi Int64) (: k Int64) (: acc Int64))
              (if (or (> lo hi) (= k 0))
                  (+ (* 100 acc) (- hi lo))
                  (let ((mid (/ (+ lo hi) 2)))
                    (let ((v (O.probe mid)))
                      (if (= v 0)
                          (+ (* 100 (+ (* 10 acc) 2)) (- hi lo))
                          (if (> v 0)
                              (search (+ mid 1) hi (- k 1) (+ (* 10 acc) 3))
                              (search lo (- mid 1) (- k 1) (+ (* 10 acc) 1))))))))
            (def (main (: n Int64))
              (handle O n
                ((probe (mid) t
                  (resume (if (= mid t) 0 (if (< mid t) 1 -1)) t)))
                (search 0 100 5 0)))
            (export main)))
  (call   main (: 37 Int64)) (output (: 13224 Int64))
  (call   main (: 50 Int64)) (output (: 300 Int64)))
