(case "bt1 a BOOL handler state TOGGLES per dispatch — three draws read the alternating flag, seeded by input parity"
  (input  (do
            (effect E (op flag (-> Int64)))
            (def (main (: n Int64))
              (handle E (= (% n 2) 0)
                ((flag () b (resume (if b 1 0) (not b))))
                (+ (* 100 (E.flag)) (+ (* 10 (E.flag)) (E.flag)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 101 Int64))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
