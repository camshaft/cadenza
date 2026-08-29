(do (def (main (: n Int64)) (match (tuple n 2) ((guard #tuple(a b) (> a 5)) 100) (#tuple(a b) (+ a b)))) (export main))
