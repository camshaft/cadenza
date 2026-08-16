(case "ek8 COMPOUND: the s-around-k shape in an INNER handler whose arm ALSO performs the OUTER effect"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ (+ s (A.get)) (k x))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))
