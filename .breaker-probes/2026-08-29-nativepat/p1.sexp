(do (def (main (: n Int64)) (match (tuple n (+ n 1)) (#tuple(a b) (+ (* a 10) b)))) (export main))
