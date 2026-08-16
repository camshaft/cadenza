(case "rv3 the inner arm's NEXT-STATE expression draws from the OUTER handler — the inner state accumulates outer draws across dispatches"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((tick () t (resume t (+ t (O.next)))))
                  (+ (* 100 (I.tick)) (+ (* 10 (I.tick)) (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64))
  (call   main (: -1 Int64)) (output (: -9 Int64)))
