(case "ps2 the seed perform ADVANCES and the inner handler's ABORT reads the seeded value"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (+ (handle B (A.get)
                     ((bail (u) t (* 100 t)))
                     (+ 7 (B.bail)))
                   (A.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
