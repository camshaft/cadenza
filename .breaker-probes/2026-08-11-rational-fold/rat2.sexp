(do (def (main (: n Int64)) (Rational.truncate (+ (Rational.of n 1) (Rational.of 1 1)))) (export main))
