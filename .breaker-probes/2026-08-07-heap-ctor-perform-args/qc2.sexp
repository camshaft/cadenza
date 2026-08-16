(case "qc2 the gcd face: a reducible num/den pair from performs canonicalizes (4/8 → 1/2)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (* s 2))))
                (let ((q (Rational.of (St.next) (St.next))))
                  (+ (* 10 (Int64.of (Rational.numerator q)))
                     (Int64.of (Rational.denominator q))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 12 Int64)))
