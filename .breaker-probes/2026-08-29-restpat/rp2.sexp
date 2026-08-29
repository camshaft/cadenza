(do (def (f (: xs (List (List Int64)))) (match xs ((list (list a .. inner) .. outer) (+ (* a 100) (+ (* (List.len inner) 10) (List.len outer)))) (_ -1)))
    (def (main (: n Int64)) (f (list (list n 5 6) (list 7) (list 8))))
    (export main))
