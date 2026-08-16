(case "eg2 simpler: scalar-arg store op building trie state across 20 resumes"
  (input  (do
            (effect Db (op store (-> Int64 Int64)) (op get (-> Int64 Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Db.store i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((store (k) s (resume 0 (Map.insert s k (* k 7))))
                 (get (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None _u) -1)) s)))
                (do
                  (feed 1 (+ n 1))
                  (Db.get 15))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 105 Int64)))
