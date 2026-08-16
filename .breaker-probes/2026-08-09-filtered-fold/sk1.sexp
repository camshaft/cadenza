(case "sk1 a FILTERED fold — only even draws accumulate but every draw advances the thread, kept-count pins the filter"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (fold (: k Int64) (: acc Int64) (: kept Int64))
              (if (<= k 0)
                  (+ (* 100 acc) (* 10 kept))
                  (let ((d (E.next)))
                    (if (= (% d 2) 0)
                        (fold (- k 1) (+ acc d) (+ kept 1))
                        (fold (- k 1) acc kept)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (fold 4 0 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 620 Int64))
  (call   main (: 1 Int64)) (output (: 620 Int64))
  (call   main (: -4 Int64)) (output (: -580 Int64)))
