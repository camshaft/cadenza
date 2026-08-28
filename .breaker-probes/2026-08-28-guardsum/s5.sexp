(do (def (f (: o (Option Int64))) (match o ((guard (Some v) (> v 3)) 100) ((Some v) v) ((None _u) -1)))
    (def (main (: n Int64)) (f (if (> n 0) (Some n) (None))))
    (export main))
