(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((q (Rational.of (St.next) (St.next))))
        (+ (* 10 (Int64.of (Rational.numerator q)))
           (Int64.of (Rational.denominator q))))))
  (export main))
