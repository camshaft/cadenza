(case "ha1 an INNER op's arguments draw from the OUTER handler — two outer draws cross the inner dispatch boundary in order"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tens (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((tens (a b) s (resume (+ (* 10 a) b) s)))
                  (I.tens (O.next) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
