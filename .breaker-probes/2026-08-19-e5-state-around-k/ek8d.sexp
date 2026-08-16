(case "ek8d dissect: s-around-k with the perform in the K-ARG ((+ s (k (+ x (A.get)))))"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ s (k (+ x (A.get))))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))
