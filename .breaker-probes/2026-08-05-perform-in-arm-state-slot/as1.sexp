(case "as1 an OUTER perform in the NEXT-STATE slot of an inner arm ((resume v (+ t (A.get))))"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume t (+ t (A.get)))))
                  (+ (* 100 (B.step)) (+ (* 10 (B.step)) (B.step))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 511 Int64)))
