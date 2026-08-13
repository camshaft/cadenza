(case "swt1 VALUE-YIELDING map ops in the arm — Map.swap answers the PRIOR value it replaced and Map.take answers the value it removed, both tuple-projected in the arm with absent-key sentinels"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64))
              (op del (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (match (Map.swap m k v)
                    ((tuple prior m2)
                      (resume (match prior ((Some p) p) ((None u) -1)) m2))))
                 (del (k) m
                  (match (Map.take m k)
                    ((tuple taken m2)
                      (resume (match taken ((Some t) t) ((None u) -1)) m2)))))
                (let ((a (S.put n n)))
                  (let ((b (S.put n 8)))
                    (let ((c (S.del n)))
                      (let ((d (S.del n)))
                        (let ((e (S.put (+ n 1) 9)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 100 (+ a 2)) (+ b 2))) (+ c 2))) (+ d 2))) (+ e 2)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10510101 Int64))
  (call   main (: 40 Int64)) (output (: 14210101 Int64)))
