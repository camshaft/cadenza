(case "ek1 LEAK-WITNESS: a ctl-arm referencing s AROUND the k-call (should decline or fold, currently CDZ0101)"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (+ s (k x))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 105 Int64)))
