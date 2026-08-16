(case "ek10 conjunction with the perform in the RESUME-STATE slot: (k x) result folds but state expr performs"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ s (k (+ x 1)))))
                  (+ (G.y 5) (A.get)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 113 Int64)))
