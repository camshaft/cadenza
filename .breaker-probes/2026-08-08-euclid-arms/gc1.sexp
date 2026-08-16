(case "gc1 a EUCLID-step arm — each dispatch advances (a,b) one gcd step, low digits of the descent spell the trace"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 18)
                ((step () s (match s
                              ((tuple a b)
                                (resume a (if (= b 0) (tuple a b) (tuple b (% a b))))))))
                (let ((d1 (E.step)))
                  (let ((d2 (E.step)))
                    (let ((d3 (E.step)))
                      (let ((d4 (E.step)))
                        (+ (* 1000 (% d1 10))
                           (+ (* 100 (% d2 10))
                              (+ (* 10 (% d3 10)) (% d4 10))))))))))
            (export main)))
  (call   main (: 48 Int64)) (output (: 8826 Int64))
  (call   main (: 21 Int64)) (output (: 1833 Int64)))
