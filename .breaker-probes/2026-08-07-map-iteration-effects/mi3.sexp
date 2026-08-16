(case "mi3 the arm AGGREGATES map values via a to-list walk — the n=1 row OVERWRITES the seed value, shrinking the total"
  (input  (do
            (effect Db (op put (-> Int64 Int64)) (op total (-> Int64)))
            (def (sum-snd (: xs (List (Tuple Int64 Int64))) (: i Int64))
              (match (List.at xs i)
                ((Some p) (match p ((tuple k v) (+ v (sum-snd xs (+ i 1))))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle Db (map (1 100))
                ((put (k) m (resume (Map.len m) (Map.insert m k k)))
                 (total () m (resume (sum-snd (Map.to-list m) 0) m)))
                (do
                  (Db.put n)
                  (Db.put 3)
                  (Db.total))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 108 Int64))
  (call   main (: 1 Int64)) (output (: 4 Int64))
  (call   main (: 3 Int64)) (output (: 103 Int64)))
