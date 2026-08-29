(do (def (f (: xs (List Int64))) (match xs ((list a .. mid b) (+ (* a 100) (+ (* (List.len mid) 10) b))) (_ -1)))
    (def (main (: n Int64)) (f (list n 5 6 7)))
    (export main))
