(do (def (f (: o (Option Int64))) (match o ((guard (Some x) (> x 5)) (* x 10)) ((Some x) x) ((None u) -1)))
    (def (main (: n Int64)) (+ (f (Some n)) (+ (f (Some 3)) (f (None)))))
    (export main))
