(do (def (main (: n Int64)) (match #tuple(n 4) ((tuple a b) (+ (* a 10) b)))) (export main))
