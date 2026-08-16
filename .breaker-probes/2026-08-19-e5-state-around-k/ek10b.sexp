(case "ek10b control: same body-performs-outer but arm WITHOUT s-around-k"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (k (+ x s))))
                  (+ (G.y 5) (A.get)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))
