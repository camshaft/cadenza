(do (def (mk (: a Int64)) (fn ((: k Int64)) (+ k a)))
    (def (main (: n Int64)) ((mk 10) n))
    (export main))
