(do (@ (requires (> x 0)) (def (f (: x Int64)) (* x 10)))
    (def (main (: n Int64)) (f n))
    (export main))
