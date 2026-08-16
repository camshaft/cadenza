(case "nb2 a Rational SQUARE over a perform draw — 1/5 squared stays exactly 1/25"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (Rational.of 1 (St.next))))
                  (let ((sq (* r r)))
                    (+ (* 10 (Int64.of (Rational.numerator sq)))
                       (Int64.of (Rational.denominator sq)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 35 Int64)))
