(case "rowmr MUTUAL pair draws then a trailing draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64))
              (if (<= k 0) 0 (+ (* 10 (E.next)) (od (- k 1)))))
            (def (od (: k Int64))
              (if (<= k 0) 0 (+ (E.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (ev 2)) (E.next))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 999999 Int64)))
