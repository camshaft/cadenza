(do (def (main (: n Int64)) (match (tuple (tuple n 2) 3) (#tuple(#tuple(a b) c) (+ (* a 100) (+ (* b 10) c))))) (export main))
