(case "ms7 Map.empty state + LOOKUP ONLY in the arm (no insert anywhere)"
  (input  (do
            (effect Db (op get (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((get (v) m (resume (match (Map.lookup m "k") ((Some x) x) ((None _u) v)) m)))
                (Db.get n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
