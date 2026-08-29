(do (def (main (: d Int64)) (match (List.at (list (tuple (/ 5 d) 1) (tuple 20 30)) 1) ((Some (tuple a b)) (+ a b)) ((None _u) -1))) (export main))
