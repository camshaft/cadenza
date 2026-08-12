(case "wp1 sibling: FLOAT computed perform arg + two lookup-matches — f64 scratch vs i32 handle"
  (input  (do
            (effect S (op put (-> Float64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m v)
                              ((Some x) (Map.insert m v v))
                              ((None u) (Map.insert m v v)))))
                    (resume (match (Map.lookup m2 v) ((Some x) x) ((None u) 0)) m2))))
                (S.put (Float64.of (+ n 1)) n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
