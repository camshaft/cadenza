(case "ms1 MINIMAL Map.empty handler state with String keys — one dispatch, one insert"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((put (v) m (resume (Map.len (Map.insert m "k" v)) (Map.insert m "k" v))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
