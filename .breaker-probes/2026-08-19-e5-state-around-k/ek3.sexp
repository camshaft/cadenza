(case "ek3 s AFTER the k-call: (+ (k x) s) folds 105 (bare-position face 2 of the E5 fix)"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (+ (k x) s)))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 105 Int64)))
