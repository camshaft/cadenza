(do (def (main (: n Int64)) (match (list n 5) (#list(h t) (+ (* h 100) t)) (_ -1))) (export main))
