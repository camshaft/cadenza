(case "ab8 a draw picks WHICH of two nested abort handlers fires — outer-abort skips the inner arm's scale and the tail draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Ob (op out (-> Int64 Int64)))
            (effect Ib (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle Ob 0
                  ((out (v) t (+ 9000 v)))
                  (+ (* 100 (handle Ib 0
                              ((out (v) t (+ 500 v)))
                              (let ((d (E.next)))
                                (if (= (% d 3) 0)
                                    (Ob.out d)
                                    (if (= (% d 3) 1) (Ib.out d) d)))))
                     (- (E.next) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64))
  (call   main (: 1 Int64)) (output (: 50101 Int64))
  (call   main (: 2 Int64)) (output (: 201 Int64))
  (call   main (: -4 Int64)) (output (: -399 Int64)))
