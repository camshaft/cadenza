(case "ms2 CONTROL Int64-keyed Map.empty handler state — same shape, default-coinciding type"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((put (v) m (resume (Map.len (Map.insert m 1 v)) (Map.insert m 1 v))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
