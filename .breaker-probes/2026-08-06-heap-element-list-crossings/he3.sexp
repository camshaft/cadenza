(case "he3 a list of RATIONALS op result — exact fractions cross resume and fold to a canonical sum"
  (input  (do
            (effect St (op parts (-> Int64 (List Rational))))
            (def (sum-r (: xs (List Rational)) (: i Int64) (: acc Rational))
              (match (List.at xs i)
                ((Some v) (sum-r xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((parts (k) s (resume (list (Rational.of 1 2) (Rational.of 1 3) (Rational.of 1 (* k 6))) s)))
                (let ((r (sum-r (St.parts n) 0 (Rational.of 0 1))))
                  (+ (* 10 (Int64.of (Rational.numerator r)))
                     (Int64.of (Rational.denominator r))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 145 Int64)))
