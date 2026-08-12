(case "ab21 sibling: the arm ABORTS (no resume) with a value built from two lookup-matches over a computed-key insert — the #21 shape on the abort path"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Map.insert Map.empty n (* n 3))
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k 5))
                              ((None u) (Map.insert m k 5)))))
                    (+ (* 100 (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)))
                       (match (Map.lookup m2 n) ((Some y) y) ((None u) 0))))))
                (S.put (+ n 1) n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 509 Int64))
  (call   main (: 10 Int64)) (output (: 530 Int64)))
