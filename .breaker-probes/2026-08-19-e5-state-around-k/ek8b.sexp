(case "ek8b control: arm performs OUTER without s-around-k (k applied directly to op result)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (k (+ x (A.get)))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 12 Int64)))
