(case "ms6 LIST-valued map state, lookup-with-fallback then push then insert (the ns1 arm, one dispatch)"
  (input  (do
            (effect Db (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((put (v) m
                  (let ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (list)))))
                    (let ((nxs (List.push xs v)))
                      (resume (List.len nxs) (Map.insert m "k" nxs))))))
                (Db.put n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
