(case "inv1 MODULAR INVERSE by extended Euclid in the arm — the iteration count is value-dependent, the Bezout coefficient normalizes through the double-mod idiom, and the body verifies v times inverse is one mod 97"
  (input  (do
            (effect S (op inv (-> Int64 Int64)))
            (def (eg (: or Int64) (: r Int64) (: os Int64) (: s Int64))
              (if (= r 0)
                  os
                  (let ((q (/ or r)))
                    (eg r (- or (* q r)) s (- os (* q s))))))
            (def (norm (: x Int64)) (% (+ (% x 97) 97) 97))
            (def (main (: n Int64))
              (handle S 0
                ((inv (v) cnt
                  (let ((x (norm (eg v 97 1 0))))
                    (resume (+ (* x 10) (+ cnt 1)) (+ cnt 1)))))
                (let ((a (S.inv n)))
                  (let ((b (S.inv 3)))
                    (let ((ia (/ a 10)))
                      (let ((chk (% (* n ia) 97)))
                        (+ (* 100000 a) (+ (* 10 b) chk))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 39106521 Int64))
  (call   main (: 10 Int64)) (output (: 68106521 Int64)))
