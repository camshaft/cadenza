(case "cr1 a registry: closures EXTRACTED from a list by index and applied (not just stored)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def fns (list (fn ((: x Int64)) (+ x 1))
                               (fn ((: x Int64)) (* x 10))
                               (fn ((: x Int64)) (- x 5))))
                (+ (* 100 (match (List.at fns 1) ((Some f) (f k)) ((None _u) -1)))
                   (match (List.at fns (% k 3)) ((Some g) (g 7)) ((None _u) -1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4070 Int64))
  (call   main (: 3 Int64)) (output (: 3008 Int64)))
