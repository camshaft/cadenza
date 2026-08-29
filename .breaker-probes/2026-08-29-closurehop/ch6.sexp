(do (def (iter f (: k Int64) (: acc Int64)) (if (<= k 0) acc (iter f (- k 1) (f acc))))
    (def (main (: n Int64)) (iter (if (> n 0) (fn ((: a Int64)) (+ a 3)) (fn ((: a Int64)) (* a 2))) 4 n))
    (export main))
