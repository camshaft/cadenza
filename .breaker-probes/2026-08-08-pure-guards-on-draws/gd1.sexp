(case "gd1 PURE guards over a draw-bound scrutinee — three guard tiers select on the drawn value, the trailing probe pins one advance"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.next)
                           ((guard x (> x 5)) (+ 100 x))
                           ((guard x (> x 0)) (+ 200 x))
                           (x (+ 300 x))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1071 Int64))
  (call   main (: 3 Int64)) (output (: 2031 Int64))
  (call   main (: -2 Int64)) (output (: 2981 Int64)))
