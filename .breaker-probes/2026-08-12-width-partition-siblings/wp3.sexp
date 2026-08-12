(case "wp3 sibling: THREE-param op, two computed args, two lookup-matches"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v w) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k (+ v w)))
                              ((None u) (Map.insert m k (+ v w))))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (S.put (+ n 1) (* n 2) (- n 1))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64)))
