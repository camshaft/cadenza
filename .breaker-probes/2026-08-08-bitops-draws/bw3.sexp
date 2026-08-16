(case "bw3 shift-up then shift-back round trip over MASKED draws — both the value and the shift count come from the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 6))))
                (let ((d (& (E.next) 15)))
                  (let ((k (& (E.next) 3)))
                    (let ((up (<< d k)))
                      (+ (* 1000 (if (= (>> up k) d) 1 5))
                         (+ (* 10 up) k)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1061 Int64))
  (call   main (: 0 Int64)) (output (: 1002 Int64))
  (call   main (: 10 Int64)) (output (: 1100 Int64)))
