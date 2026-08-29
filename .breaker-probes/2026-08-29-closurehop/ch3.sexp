(do (def (main (: n Int64)) (List.fold (list 1 2 3) n (fn ((: acc Int64) (: e Int64)) (+ acc (* e 2)))))
    (export main))
