(case "ra1 a recursive walk BAILS mid-descent when a draw crosses a limit — the abort tears down the partial recursion, the outer thread keeps the draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (let ((d (E.next)))
                    (if (> d 7)
                        (Bail.out d)
                        (+ d (walk (- k 1)))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out (v) t v))
                            (walk 5)))
                   (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 809 Int64))
  (call   main (: 0 Int64)) (output (: 1005 Int64))
  (call   main (: 20 Int64)) (output (: 2021 Int64)))
