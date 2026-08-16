(case "hn2 a bare RATIONAL as op ARGUMENT — the arm reads exact numerator/denominator off the crossed value"
  (input  (do
            (effect St (op mix (-> Rational Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((mix (r) s
                  (let ((q (+ r (Rational.of 1 6))))
                    (resume (+ (* 10 (Int64.of (Rational.numerator q)))
                               (Int64.of (Rational.denominator q)))
                            s))))
                (St.mix (Rational.of 1 (- n 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))
