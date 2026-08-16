(case "mr4 an ACCUMULATOR threads the mutual pair — ev doubles it plus a draw, od triples it plus a draw, alternating scales"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64) (: acc Int64))
              (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next)))))
            (def (od (: k Int64) (: acc Int64))
              (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 4 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 71 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64))
  (call   main (: -3 Int64)) (output (: -69 Int64)))
