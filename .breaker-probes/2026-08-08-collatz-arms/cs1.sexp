(case "cs1 a COLLATZ-step arm — even states halve, odd states triple-plus-one, low digits of four reads trace the orbit"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((step () s (resume s (if (= (% s 2) 0) (/ s 2) (+ (* 3 s) 1)))))
                (let ((d1 (E.step)))
                  (let ((d2 (E.step)))
                    (let ((d3 (E.step)))
                      (let ((d4 (E.step)))
                        (+ (* 1000 (% d1 10))
                           (+ (* 100 (% d2 10))
                              (+ (* 10 (% d3 10)) (% d4 10))))))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 6305 Int64))
  (call   main (: 7 Int64)) (output (: 7214 Int64))
  (call   main (: 5 Int64)) (output (: 5684 Int64)))
