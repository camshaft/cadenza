(case "mr6 TWO calls into the mutual pair under one handler"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64) (: acc Int64))
              (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next)))))
            (def (od (: k Int64) (: acc Int64))
              (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (ev 2 0)) (ev 2 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 513 Int64))
  (call   main (: 0 Int64)) (output (: 109 Int64))
  (call   main (: -2 Int64)) (output (: -699 Int64)))
