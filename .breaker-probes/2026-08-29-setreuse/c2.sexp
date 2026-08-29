(do (def (main (: n Int64)) (do (def s (Set.of (list n 2 3))) (def s2 (Set.insert s 9)) (+ (Set.len s) (Set.len s2))))
    (export main))
