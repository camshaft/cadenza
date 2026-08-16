(case "rq3 a FRACTIONAL drain with a sign guard — subtracting 1/3 per tick until the rational state crosses zero, the recursion counts full ticks"
  (input  (do
            (effect E (op drain (-> Int64)))
            (def (spin (: ticks Int64))
              (let ((sig (E.drain)))
                (if (< sig 0) (+ (* 100 ticks) sig) (spin (+ ticks 1)))))
            (def (main (: n Int64))
              (handle E (Rational.of n 3)
                ((drain () s
                  (let ((s2 (- s (Rational.of 1 3))))
                    (resume (if (< s2 (Rational.of 0 1)) (Int64.of (Rational.numerator s2)) 0) s2))))
                (spin 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 199 Int64))
  (call   main (: 1 Int64)) (output (: 99 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
