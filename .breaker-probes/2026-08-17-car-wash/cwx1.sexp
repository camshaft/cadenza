(case "cwx1 a CAR WASH with a wax upsell — a wash needs three soap (else a nine-hundred refusal showing the dregs), the wax takes only a JUST-washed car (washed strictly ahead of waxed, else an eight-hundred refusal), the read packs soap washed and waxed, and the seed's soap float lets one run wash-wash-wax-wash-wax cleanly while the other fails its second wash so the SAME wax call is served on one run and refused on the other"
  (input  (do
            (effect W
              (op wash (-> Int64))
              (op wax (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (+ (: 4 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((wash () st
                  (match st
                    ((tuple soap washed waxed)
                      (if (>= soap 3)
                          (resume (+ (* (- soap 3) 10) 1)
                                  (tuple (- soap 3) (+ washed 1) waxed))
                          (resume (+ (: 900 Int64) soap) st)))))
                 (wax (pay) st
                  (match st
                    ((tuple soap washed waxed)
                      (if (> washed waxed)
                          (resume (+ (* (+ waxed 1) 10) (% pay 10))
                                  (tuple soap washed (+ waxed 1)))
                          (resume (+ (: 800 Int64) waxed) st)))))
                 (read () st
                  (match st
                    ((tuple soap washed waxed)
                      (resume (+ (* soap 100) (+ (* washed 10) waxed)) st)))))
                (let ((a (W.wash)))
                  (let ((b (W.wash)))
                    (let ((c (W.wax (: 5 Int64))))
                      (let ((d (W.wash)))
                        (let ((e (W.wax (: 7 Int64))))
                          (let ((f (W.read)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 41011015901027122 Int64))
  (call   main (: 0 Int64)) (output (: 11901015901801111 Int64)))
