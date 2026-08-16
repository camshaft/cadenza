(case "stm1 a POSTAGE METER with weight bands and an ink tank — franking costs five up to twenty grams plus two per STARTED ten grams over (a ceiling-division band), an under-inked frank REJECTS with the quoted postage and no state change, audit packs ink franked and rejected, and the seed sizes the tank so one run franks all three parcels while the other rejects the heavy one"
  (input  (do
            (effect P
              (op frank (-> Int64 Int64))
              (op audit (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (+ (: 20 Int64) (* (% n 3) 15)) (: 0 Int64) (: 0 Int64))
                ((frank (g) st
                  (match st
                    ((tuple ink fr rj)
                      (if (<= g 20)
                          (if (< ink 5)
                              (resume (: 905 Int64) (tuple ink fr (+ rj 1)))
                              (resume (+ (* 5 10) (% (- ink 5) 10)) (tuple (- ink 5) (+ fr 1) rj)))
                          (if (< ink (+ 5 (* (/ (+ (- g 20) 9) 10) 2)))
                              (resume (+ (: 900 Int64) (+ 5 (* (/ (+ (- g 20) 9) 10) 2))) (tuple ink fr (+ rj 1)))
                              (resume (+ (* (+ 5 (* (/ (+ (- g 20) 9) 10) 2)) 10) (% (- ink (+ 5 (* (/ (+ (- g 20) 9) 10) 2))) 10)) (tuple (- ink (+ 5 (* (/ (+ (- g 20) 9) 10) 2))) (+ fr 1) rj)))))))
                 (audit () st
                  (match st ((tuple ink fr rj) (resume (+ (* ink 100) (+ (* fr 10) rj)) st)))))
                (let ((a (P.frank (: 15 Int64))))
                  (let ((b (P.frank (: 45 Int64))))
                    (let ((c (P.frank (: 80 Int64))))
                      (let ((f (P.audit)))
                        (+ (* 10000 (+ (* 10000 (+ (* 10000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 50011901720230 Int64))
  (call   main (: 0 Int64)) (output (: 55011409170421 Int64)))
