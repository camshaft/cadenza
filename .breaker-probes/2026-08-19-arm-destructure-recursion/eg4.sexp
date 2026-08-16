(case "eg4 tuple-arg arm match-destructure but STRAIGHT-LINE feed (no recursion)"
  (input  (do
            (effect Db (op store (-> (Tuple Int64 Int64) Int64)) (op get (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((store (p) s (match p ((tuple k v) (resume 0 (Map.insert s k v)))))
                 (get (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None _u) -1)) s)))
                (do
                  (Db.store (tuple 15 105))
                  (Db.get 15))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 105 Int64)))
