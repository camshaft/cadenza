(case "rx2 cross-denominator RATIONAL accumulation lands exactly at unity — halves plus thirds plus sixths, the verdict fires only on the last dispatch"
  (input  (do
            (effect R (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle R (Rational.of 0 1)
                ((step (d) s
                  (let ((nxt (+ s (Rational.of 1 d))))
                    (resume (if (= nxt (Rational.of 1 1)) 1 0) nxt))))
                (+ (R.step 2) (+ (* 10 (R.step 3)) (* 100 (R.step 6))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
