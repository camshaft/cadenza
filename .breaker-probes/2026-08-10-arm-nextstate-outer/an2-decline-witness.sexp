(case "an2 the inner arm's NEXT-STATE draws the OUTER effect — each inner dispatch re-seeds its state from the outer thread"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((tick () t (resume t (O.next))))
                  (+ (I.tick) (+ (* 10 (I.tick)) (* 100 (I.tick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 650 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64))
  (call   main (: -2 Int64)) (output (: -120 Int64)))
