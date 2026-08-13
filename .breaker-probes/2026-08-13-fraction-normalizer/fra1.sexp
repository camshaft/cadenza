(case "fra1 a LOWEST-TERMS fraction state — each add cross-multiplies then renormalizes by an in-arm Euclid gcd, the n=1 seed collapses to exactly 1/1 mid-run"
  (input  (do
            (effect S (op addf (-> Int64 Int64 Int64)))
            (def (gcd (: a Int64) (: b Int64))
              (if (= b 0) a (gcd b (% a b))))
            (def (main (: n Int64))
              (handle S (let ((g (gcd n 4))) (tuple (/ n g) (/ 4 g)))
                ((addf (a b) st
                  (match st
                    ((tuple num den)
                      (let ((nn (+ (* num b) (* a den))))
                        (let ((nd (* den b)))
                          (let ((g (gcd nn nd)))
                            (let ((n2 (/ nn g)))
                              (let ((d2 (/ nd g)))
                                (resume (+ (* 100 n2) d2) (tuple n2 d2)))))))))))
                (let ((a (S.addf 1 4)))
                  (let ((b (S.addf 1 2)))
                    (let ((c (S.addf 1 6)))
                      (+ (* 10000 (+ (* 10000 a) b)) c))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 10201010706 Int64))
  (call   main (: 2 Int64)) (output (: 30405041712 Int64)))
