(case "dp2 a tuple-pattern param receiving another FUNCTION'S tuple RESULT (producer→pattern chain)"
  (input  (do
            (def (mk (: x Int64)) (tuple x (* x 3)))
            (def (add (tuple a b)) (+ a b))
            (def (main (: x Int64)) (add (mk x)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20 Int64)))
