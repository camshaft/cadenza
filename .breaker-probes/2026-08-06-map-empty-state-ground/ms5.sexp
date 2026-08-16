(case "ms5 scalar-valued map state, LOOKUP with fallback before insert"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((put (v) m
                  (let ((cur (match (Map.lookup m "k") ((Some x) x) ((None _u) 0))))
                    (resume (+ cur v) (Map.insert m "k" (+ cur v))))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
