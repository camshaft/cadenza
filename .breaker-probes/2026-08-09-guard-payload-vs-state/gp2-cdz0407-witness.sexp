(case "gp2 the guard's condition PERFORMS an OUTER effect from inside the inner handler's arm — a drawing guard routes admit/reject"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op judge (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((judge (v) t
                    (match v
                      ((guard x (> x (O.next))) (resume 1 t))
                      (_x (resume 0 t)))))
                  (+ (* 10 (I.judge 3)) (I.judge 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
