(case "rx1 a RATIONAL state accumulates thirds exactly — the times-three round-trip verdict is true only at the seed"
  (input  (do
            (effect R (op frac (-> Int64)))
            (def (main (: n Int64))
              (handle R (Rational.of 1 3)
                ((frac () s
                  (resume (if (= (* s (Rational.of 3 1)) (Rational.of 1 1)) 1 0)
                          (+ s (Rational.of 1 3)))))
                (+ (R.frac) (* 10 (R.frac)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
