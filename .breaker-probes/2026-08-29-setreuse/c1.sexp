(do (def (main (: n Int64)) (do (def s (Set.of (list 1 2 3))) (def hit (Set.contains s n)) (def s2 (Set.insert s 9)) (+ (if hit 1 0) (Set.len s2))))
    (export main))
