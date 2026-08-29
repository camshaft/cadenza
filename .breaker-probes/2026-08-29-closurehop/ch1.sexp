(do (def (apply2 f (: x Int64)) (f (f x)))
    (def (main (: n Int64)) (apply2 (fn ((: k Int64)) (+ k 3)) n))
    (export main))
