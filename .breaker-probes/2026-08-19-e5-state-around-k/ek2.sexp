(case "ek2 CONTROL: s INSIDE the k-arg folds (the covered side of the E5 s-scoping boundary)"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (k (+ x s))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 105 Int64)))
