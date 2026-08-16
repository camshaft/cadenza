(case "flt1 a TWO-STAGE sediment filter — pass traps the value's residue mod the seed then HALF of what remains answering the survivor, backwash resets the trap answering its contents, and the finer mesh traps MORE in stage one leaving less for stage two so the survivors differ while the last backwash CONVERGES"
  (input  (do
            (effect F
              (op pass (-> Int64 Int64))
              (op backwash (-> Int64)))
            (def (main (: n Int64))
              (handle F (: 0 Int64)
                ((pass (v) trapped
                  (match (% v (+ (% n 4) 2))
                    (r1
                      (match (/ (- v r1) 2)
                        (r2
                          (resume (- (- v r1) r2) (+ trapped (+ r1 r2))))))))
                 (backwash () trapped (resume trapped 0)))
                (let ((a (F.pass 23)))
                  (let ((b (F.pass 14)))
                    (let ((c (F.backwash)))
                      (let ((d (F.pass 9)))
                        (let ((e (F.backwash)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1006210405 Int64))
  (call   main (: 0 Int64)) (output (: 1107190405 Int64)))
