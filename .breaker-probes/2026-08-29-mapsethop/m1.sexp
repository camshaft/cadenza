(do (def (main (: n Int64)) (match (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 2 20) n) ((Some v) v) ((None _u) -1)))
    (export main))
