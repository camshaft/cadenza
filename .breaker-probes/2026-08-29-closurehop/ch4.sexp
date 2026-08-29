(do (def (main (: n Int64)) ((if (> n 0) (fn ((: k Int64)) (+ k 1)) (fn ((: k Int64)) (* k 2))) 5))
    (export main))
