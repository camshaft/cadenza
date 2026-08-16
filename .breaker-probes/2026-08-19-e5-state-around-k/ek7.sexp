(case "ek7 ESCALATION: TWO k-calls with s around both (multi-shot continuation + sibling state)"
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G n
                ((y (x) s k (+ s (+ (k x) (k (+ x 1))))))
                (G.y 5)))
            (export main)))
  (call   main (: 100 Int64)) (output (: 111 Int64)))
