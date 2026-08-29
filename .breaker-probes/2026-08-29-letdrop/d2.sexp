(do (def (main (: n Int64)) (do (def r1 (list n 2)) (def r2 (if (> n 0) (list r1) (list r1 (list 9)))) (List.len r2))) (export main))
