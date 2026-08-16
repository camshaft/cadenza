(case "cp2 BINARY SEARCH against a hidden state target — the body bisects on the arm's ordering verdicts, eight probes find any target in 0..100"
  (input  (do
            (effect S (op cmp (-> Int64 Int64)))
            (def (bisect (: lo Int64) (: hi Int64) (: k Int64))
              (if (< k 1) -1
                (let ((mid (/ (+ lo hi) 2)))
                  (let ((c (S.cmp mid)))
                    (if (= c 0) mid
                      (if (< c 0) (bisect lo (- mid 1) (- k 1))
                        (bisect (+ mid 1) hi (- k 1))))))))
            (def (main (: n Int64))
              (handle S n
                ((cmp (v) s (resume (if (< v s) 1 (if (> v s) -1 0)) s)))
                (bisect 0 100 8)))
            (export main)))
  (call   main (: 37 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 100 Int64)) (output (: 100 Int64))
  (call   main (: 63 Int64)) (output (: 63 Int64)))
