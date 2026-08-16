(case "mr5 TWENTY alternating mutual levels with the scaling accumulator — depth stress on the cross-function fold"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64) (: acc Int64))
              (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next)))))
            (def (od (: k Int64) (: acc Int64))
              (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 20 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 79815335 Int64))
  (call   main (: 0 Int64)) (output (: 31442395 Int64))
  (call   main (: -10 Int64)) (output (: -452287005 Int64)))
