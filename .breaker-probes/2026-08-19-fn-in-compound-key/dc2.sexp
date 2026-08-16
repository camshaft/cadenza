(case "dc2 a RECORD containing a closure as a map key declines CDZ0216"
  (input  (do
            (def (main (: n Int64))
              (do
                (def f (fn ((: x Int64)) (+ x n)))
                (Map.len (Map.insert Map.empty (record (id 1) (cb f)) 42))))
            (export main)))
  (error  CDZ0216))
