(case "ms8 NON-EMPTY seeded state + the same lookup-fallback-insert arm as ms5"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db (Map.insert Map.empty "seed" 0)
                ((put (v) m
                  (let ((cur (match (Map.lookup m "k") ((Some x) x) ((None _u) 0))))
                    (resume (+ cur v) (Map.insert m "k" (+ cur v))))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
