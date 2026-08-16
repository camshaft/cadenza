(case "ps1 an INNER handle's seed is computed by performing the OUTER effect ((handle B (A.get) ...))"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op read (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (+ (handle B (A.get)
                     ((read (u) t (resume t t)))
                     (B.read))
                   (A.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
