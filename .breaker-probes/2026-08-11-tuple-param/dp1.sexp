(case "dp1 a tuple-pattern param destructuring a tuple pulled FROM A MAP slot"
  (input  (do
            (def (add (tuple a b)) (+ a b))
            (def (main (: x Int64))
              (match (Map.lookup (Map.insert Map.empty 1 (tuple x (* x 2))) 1)
                ((Some p) (add p))
                ((None _u) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))
