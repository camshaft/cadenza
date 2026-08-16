(case "ek5 LET-BOUND k-result face: (def r (k x)) then (+ r s) — distinct pre-existing rejection, should fold 105"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (do (def r (k x)) (+ r s))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 105 Int64)))
