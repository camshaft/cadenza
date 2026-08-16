(case "ek4 the OP-PARAM x reused around the k-call: (+ x (k x)) folds 10 (bare-position face 3)"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (+ x (k x))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 10 Int64)))
