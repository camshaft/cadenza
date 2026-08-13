(case "mki1 a MAP+KEYSET invariant pair — every put and remove maintains set = keys(map) across both structures in one transition, and paired membership checks verify both sides agree; the n=5 seed collapses the two puts into one key"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64))
              (op rm (-> Int64 Int64))
              (op chk (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple Map.empty (Set.of (list)))
                ((put (k v) st
                  (match st
                    ((tuple m ks)
                      (let ((m2 (Map.insert m k v)))
                        (let ((ks2 (Set.insert ks k)))
                          (resume (+ (* 10 (Map.len m2)) (Set.len ks2)) (tuple m2 ks2)))))))
                 (rm (k) st
                  (match st
                    ((tuple m ks)
                      (let ((m2 (Map.remove m k)))
                        (let ((ks2 (Set.remove ks k)))
                          (resume (+ (* 10 (Map.len m2)) (Set.len ks2)) (tuple m2 ks2)))))))
                 (chk (k) st
                  (match st
                    ((tuple m ks)
                      (resume (+ (* 10 (match (Map.lookup m k) ((Some _v) 1) ((None _u) 0)))
                                 (if (Set.contains ks k) 1 0))
                              st)))))
                (let ((a (S.put n 1)))
                  (let ((b (S.put 5 2)))
                    (let ((c (S.rm n)))
                      (let ((d (S.chk n)))
                        (let ((e (S.chk 5)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1122110011 Int64))
  (call   main (: 5 Int64)) (output (: 1111000000 Int64)))
