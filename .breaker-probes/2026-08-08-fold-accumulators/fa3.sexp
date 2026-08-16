(case "fa3 each fold level draws TWICE and mixes them asymmetrically — 10*first + second pins the intra-level draw order"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ acc (+ (* 10 (E.next)) (E.next))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (fold 3 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 102 Int64))
  (call   main (: 0 Int64)) (output (: 69 Int64))
  (call   main (: -3 Int64)) (output (: -30 Int64)))
