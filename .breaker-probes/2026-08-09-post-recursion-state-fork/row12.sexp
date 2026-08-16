(case "row12 isolated"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (* 100 d) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((w (walk 0)))
                  (+ w (E.next)))))
            (export main)))
  (call   main (: 12 Int64)) (output (: 1415 Int64)))
