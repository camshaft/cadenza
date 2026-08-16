(case "tu2 a RECORD holding two tries updates one field's trie without touching the sibling"
  (input  (do
            (def (fill (: i Int64) (: k Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) k (Map.insert m (+ (* k 100) i) i))))
            (def (main (: n Int64))
              (do
                (def st (record (users (fill n 1 Map.empty)) (groups (fill n 2 Map.empty))))
                (def st2 (Record.with st #"users" (Map.insert (. st users) 999 7)))
                (+ (* 100 (match (Map.lookup (. st2 users) 999) ((Some v) v) ((None _u) -1)))
                   (+ (* 10 (Map.len (. st2 groups)))
                      (if (= (Map.len (. st users)) n) 1 0)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 1001 Int64)))
