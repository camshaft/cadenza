(case "eg3 tuple-arg arm with PROJECTION instead of match-destructure"
  (input  (do
            (effect Db (op store (-> (Tuple Int64 Int64) Int64)) (op get (-> Int64 Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Db.store (tuple i (* i 7))) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((store (p) s (resume 0 (Map.insert s (. p 0) (. p 1))))
                 (get (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None _u) -1)) s)))
                (do
                  (feed 1 (+ n 1))
                  (Db.get 15))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 105 Int64)))
