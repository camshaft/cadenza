(case "nmm1 a TWO-LEVEL Map state — group puts lookup-modify-reinsert the inner CHAMP, answers pack both level sizes, reads route through both levels with a miss sentinel"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64 Int64))
              (op get (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (: Map.empty (Map Int64 (Map Int64 Int64)))
                ((put (g k v) m
                  (let ((inner (match (Map.lookup m g) ((Some i) i) ((None u) (: Map.empty (Map Int64 Int64))))))
                    (let ((i2 (Map.insert inner k v)))
                      (let ((m2 (Map.insert m g i2)))
                        (resume (+ (* 10 (Map.len m2)) (Map.len i2)) m2)))))
                 (get (g k) m
                  (resume (match (Map.lookup m g)
                            ((Some i) (match (Map.lookup i k) ((Some v) v) ((None u) -1)))
                            ((None u) -1))
                          m)))
                (let ((a (S.put n 1 n)))
                  (let ((b (S.put n 2 9)))
                    (let ((c (S.put (+ n 1) 1 4)))
                      (let ((d (S.get n 1)))
                        (let ((e (S.get (+ n 1) 9)))
                          (+ (* 10 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) (+ e 2)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 111221031 Int64))
  (call   main (: 0 Int64)) (output (: 111221001 Int64)))
