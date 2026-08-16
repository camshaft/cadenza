(case "mt2 a RECORD handler state (table + counter) evolves both fields across resumes"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op stats (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (record (tbl Map.empty) (ops 0))
                ((put (v) s (resume 0 (record (tbl (Map.insert (. s tbl) v v)) (ops (+ (. s ops) 1)))))
                 (stats (u) s (resume (+ (* 10 (Map.len (. s tbl))) (. s ops)) s)))
                (do
                  (St.put 5)
                  (St.put 6)
                  (St.put 5)
                  (St.stats))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 23 Int64)))
