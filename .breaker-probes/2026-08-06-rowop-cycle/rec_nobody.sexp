(do (def (main) (match (record (x 3)) ((record (nope a)) 0))) (export main))
