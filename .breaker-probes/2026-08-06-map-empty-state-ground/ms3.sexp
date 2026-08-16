(case "ms3 CONTROL non-empty initial state (one pre-inserted String key) — same arm"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db (Map.insert Map.empty "seed" 0)
                ((put (v) m (resume (Map.len (Map.insert m "k" v)) (Map.insert m "k" v))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))
