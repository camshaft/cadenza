(case "ha4 an inner op's arguments dispatch to the SAME inner handler — same-effect draws inside the op's own arg list, then an outer draw"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op get (-> Int64)) (op tens (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (handle I 3
                     ((get () m (resume m (* 2 m)))
                      (tens (a b) m (resume (+ (* 10 a) b) m)))
                     (I.tens (I.get) (I.get)))
                   (* 100 (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 536 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64)))
