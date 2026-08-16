(case "ra1 a RUNTIME Rational fold: sum of 1/(k(k+1)) telescopes to n/(n+1) exactly"
  (input  (do
            (def (tele (: k Int64) (: n Int64) (: acc Rational))
              (if (> k n) acc
                  (tele (+ k 1) n (+ acc (Rational.of 1 (* k (+ k 1)))))))
            (def (main (: n Int64))
              (if (= (tele 1 n (Rational.of 0 1)) (Rational.of n (+ n 1))) 1 0))
            (export main)))
  (call   main (: 20 Int64)) (output (: 1 Int64))
  (call   main (: 7 Int64)) (output (: 1 Int64)))
