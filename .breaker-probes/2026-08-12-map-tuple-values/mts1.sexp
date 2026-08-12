(case "mts1 a Map whose VALUES are TUPLES — per-key (count,sum) stats update through tuple rebuild inside the arm, the answer packs the fresh pair"
  (input  (do
            (effect S (op obs (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (: Map.empty (Map Int64 (Tuple Int64 Int64)))
                ((obs (k v) m
                  (let ((pair (match (Map.lookup m k)
                                ((Some p) (match p ((tuple c s) (tuple (+ c 1) (+ s v)))))
                                ((None u) (tuple 1 v)))))
                    (match pair
                      ((tuple c2 s2)
                        (resume (+ (* 100 c2) s2) (Map.insert m k pair)))))))
                (let ((a (S.obs n 4)))
                  (let ((b (S.obs n n)))
                    (let ((c (S.obs (+ n 1) 9)))
                      (+ a (+ (* 1000 b) (* 1000000 c))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 109207104 Int64))
  (call   main (: 7 Int64)) (output (: 109211104 Int64)))
