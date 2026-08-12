(case "rot1 a ROTATION-CIPHER state — enc shifts a letter index with the double-mod negative-normalization idiom in the arm, tune drives the shift NEGATIVE and the normalization recovers the canonical class"
  (input  (do
            (effect S
              (op enc (-> Int64 Int64))
              (op tune (-> Int64 Int64)))
            (def (norm (: x Int64))
              (% (+ (% x 26) 26) 26))
            (def (main (: n Int64))
              (handle S n
                ((enc (i) sh (resume (norm (+ i sh)) sh))
                 (tune (d) sh
                  (let ((sh2 (+ sh d)))
                    (resume (norm sh2) sh2))))
                (let ((a (S.enc 3)))
                  (let ((b (S.tune -30)))
                    (let ((c (S.enc 3)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 80104 Int64))
  (call   main (: 0 Int64)) (output (: 32225 Int64)))
