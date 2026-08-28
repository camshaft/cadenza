(do (def (f (: o (Option Int64)) (: n Int64)) (match o ((guard (Some x) (> x n)) 1) ((Some x) 2) ((None u) 3)))
    (def (main (: n Int64)) (+ (* 100 (f (Some 10) n)) (+ (* 10 (f (Some 1) n)) (f (None) n))))
    (export main))
