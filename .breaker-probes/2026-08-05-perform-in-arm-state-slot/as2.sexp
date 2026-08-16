(case "as2 dissect: ONE B.step then observe A directly (how many times did A advance?)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume t (+ t (A.get)))))
                  (+ (* 10 (B.step)) (A.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
