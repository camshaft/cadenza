(do (def (main (: n Int64)) (match (Char.from-int (+ n 60)) ((Some c) (Char.to-int c)) ((None _u) -1))) (export main))
