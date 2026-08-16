(case "rowacc5"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (+ (* 100 d) (* 10 k)) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (walk 0) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 728 Int64)))
