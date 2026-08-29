(do (def (main (: n Int64)) (if (Set.contains (Set.of (list 1 2 3)) n) 1 0))
    (export main))
