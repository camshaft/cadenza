(case "ms4 Map-of-LISTS value type, insert only (no lookup)"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((put (v) m (resume (Map.len (Map.insert m "k" (list v))) (Map.insert m "k" (list v)))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
