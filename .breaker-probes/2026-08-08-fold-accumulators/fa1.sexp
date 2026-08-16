(case "fa1 a FOLD-style accumulator threads through a performing recursion — acc doubles then absorbs each level's draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ (* 2 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (fold 3 0)) (E.next))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 114 Int64))
  (call   main (: 0 Int64)) (output (: 43 Int64))
  (call   main (: -2 Int64)) (output (: -99 Int64)))
