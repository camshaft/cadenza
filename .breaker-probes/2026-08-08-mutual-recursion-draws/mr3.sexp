(case "mr3 the NEXT callee in the mutual group is picked by draw parity — the descent path itself follows the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (let ((d (E.next)))
                    (if (= (% d 2) 0)
                        (+ (* 10 d) (a (- k 1)))
                        (+ d (b (- k 1)))))))
            (def (a (: k Int64)) (+ 1000 (walk k)))
            (def (b (: k Int64)) (+ 2000 (walk k)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (walk 3)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4063 Int64))
  (call   main (: 1 Int64)) (output (: 5024 Int64))
  (call   main (: -4 Int64)) (output (: 3937 Int64)))
