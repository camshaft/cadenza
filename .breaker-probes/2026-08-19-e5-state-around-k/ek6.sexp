(case "ek6 ESCALATION: s around k with the k-call NESTED two ops deep in the arm expr"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (+ s (* 2 (k (+ x 1))))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))
