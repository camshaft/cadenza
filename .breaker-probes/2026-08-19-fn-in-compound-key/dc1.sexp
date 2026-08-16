(case "dc1 a map keyed by a TUPLE containing a closure declines (fn is not keyable)"
  (input  (do
            (def (main (: n Int64))
              (do
                (def f (fn ((: x Int64)) (+ x n)))
                (Map.len (Map.insert Map.empty (tuple 1 f) 42))))
            (export main)))
  (error  CDZ0216))
