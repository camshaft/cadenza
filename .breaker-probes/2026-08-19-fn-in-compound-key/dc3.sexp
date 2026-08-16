(case "dc3 a LIST of closures as a set element declines CDZ0216"
  (input  (do
            (def (main (: n Int64))
              (Set.len (Set.of (list (list (fn ((: x Int64)) (+ x n)))))))
            (export main)))
  (error  CDZ0216))
