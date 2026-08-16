(case "qc1 Rational.of over TWO perform results — ctor-arg order and gcd-canonicalization compose"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((q (Rational.of (St.next) (St.next))))
                  (+ (* 10 (Int64.of (Rational.numerator q)))
                     (Int64.of (Rational.denominator q))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 45 Int64)))
