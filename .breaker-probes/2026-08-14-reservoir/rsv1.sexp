(case "rsv1 DETERMINISTIC reservoir sampling — an LCG threaded beside the reservoir decides keep-or-replace by count-modulus, seeds route which offers displace the kept element"
  (input  (do
            (effect S (op offer (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple n -1 0)
                ((offer (v) st
                  (match st
                    ((tuple seed kept count)
                      (let ((c2 (+ count 1)))
                        (let ((s2 (% (+ (* seed 13) 7) 101)))
                          (let ((k2 (if (= (% s2 c2) 0) v kept)))
                            (resume k2 (tuple s2 k2 c2)))))))))
                (let ((a (S.offer 10)))
                  (let ((b (S.offer 20)))
                    (let ((c (S.offer 30)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 102020 Int64))
  (call   main (: 7 Int64)) (output (: 101030 Int64)))
