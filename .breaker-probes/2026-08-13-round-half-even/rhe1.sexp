(case "rhe1 a ROUND-HALF-TO-EVEN halving accumulator — each dispatch adds v/2 rounded half-to-even, the parity of the truncated quotient decides which halves bump"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (rhe (: x Int64))
              (let ((q (/ x 2)))
                (if (= (% x 2) 0)
                    q
                    (if (= (% q 2) 0) q (+ q 1)))))
            (def (main (: n Int64))
              (handle S 0
                ((add (v) s
                  (let ((s2 (+ s (rhe v))))
                    (resume s2 s2))))
                (let ((a (S.add n)))
                  (let ((b (S.add 5)))
                    (let ((c (S.add 7)))
                      (let ((d (S.add 2)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2040809 Int64))
  (call   main (: 6 Int64)) (output (: 3050910 Int64)))
