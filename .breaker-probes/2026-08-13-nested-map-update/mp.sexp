(do (def (main) (match (record (x 3)) ((record (nope a)) a))) (export main))
