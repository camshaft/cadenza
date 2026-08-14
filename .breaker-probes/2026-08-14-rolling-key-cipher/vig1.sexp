(case "vig1 a VIGENERE-style rolling-key cipher — enc adds and dec subtracts the key byte selected by the ADVANCING key index mod 3, both mod 26, so interleaved enc/dec draws consume ONE shared key stream and the seed shapes the middle key byte"
  (input  (do
            (effect V
              (op encb (-> Int64 Int64))
              (op decb (-> Int64 Int64)))
            (def (keyat (: n Int64) (: i Int64))
              (match (% i 3)
                (0 3)
                (1 (+ (% n 4) 1))
                (_ 7)))
            (def (main (: n Int64))
              (handle V (: 0 Int64)
                ((encb (b) ki
                  (resume (% (+ b (keyat n ki)) 26) (+ ki 1)))
                 (decb (b) ki
                  (resume (% (+ (- b (keyat n ki)) 26) 26) (+ ki 1))))
                (let ((a (V.encb 7)))
                  (let ((b (V.encb 24)))
                    (let ((c (V.decb 4)))
                      (let ((d (V.encb 0)))
                        (let ((e (V.decb 19)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1001230316 Int64))
  (call   main (: 0 Int64)) (output (: 1025230318 Int64)))
