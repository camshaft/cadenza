(case "zgz1 a ZIGZAG codec accumulator — enc maps signed to unsigned (2v for non-negative, -2v-1 for negative) and folds the code into the state sum, dec UN-zigzags the accumulated sum whose parity decides the sign, and the seeds put the decode answers on opposite signs"
  (input  (do
            (effect Z
              (op enc (-> Int64 Int64))
              (op dec (-> Int64)))
            (def (main (: n Int64))
              (handle Z (: 0 Int64)
                ((enc (v) s
                  (let ((z (if (< v 0) (- (* -2 v) 1) (* 2 v))))
                    (resume z (+ s z))))
                 (dec () s
                  (if (= (% s 2) 0)
                      (resume (/ s 2) s)
                      (resume (- 0 (/ (+ s 1) 2)) s))))
                (let ((a (Z.enc (- n 3))))
                  (let ((b (Z.enc 4)))
                    (let ((c (Z.dec)))
                      (let ((d (Z.enc (- 0 (+ n 1)))))
                        (let ((e (Z.dec)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1408112078 Int64))
  (call   main (: 0 Int64)) (output (: 507930107 Int64)))
