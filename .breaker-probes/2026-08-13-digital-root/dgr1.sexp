(case "dgr1 a DIGITAL-ROOT arm — nested fixed-point recursion (repeat digit-sum until single digit) runs wholly inside each dispatch, the accumulator's low digit rides along in the answer"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (dsum (: x Int64) (: acc Int64))
              (if (= x 0) acc (dsum (/ x 10) (+ acc (% x 10)))))
            (def (droot (: x Int64))
              (if (< x 10) x (droot (dsum x 0))))
            (def (main (: n Int64))
              (handle S 0
                ((feed (v) acc
                  (let ((r (droot v)))
                    (let ((a2 (+ acc r)))
                      (resume (+ (* r 10) (% a2 10)) a2)))))
                (let ((a (S.feed n)))
                  (let ((b (S.feed 999)))
                    (let ((c (S.feed 38)))
                      (+ (* 100 (+ (* 100 a) b)) c))))))
            (export main)))
  (call   main (: 47 Int64)) (output (: 229123 Int64))
  (call   main (: 5 Int64)) (output (: 559426 Int64)))
