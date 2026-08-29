(do (def (main (: n Int64)) (do (def m (Map.insert (Map.empty) n 10)) (def m2 (Map.insert m 2 20)) (+ (Map.len m) (Map.len m2))))
    (export main))
