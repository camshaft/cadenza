(case "qy2 a Qty-keyed map churned with differently-normalized magnitudes equals the direct build"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (Qty.of (Rational.of (* i 2) 4) (Unit.base #"meter")) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (Qty.of (Rational.of i 2) (Unit.base #"meter"))))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert Map.empty (Qty.of (Rational.of 999 1) (Unit.base #"meter")) 50))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned (Qty.of (Rational.of 1998 2) (Unit.base #"meter"))) ((Some v) (if (= v 50) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 11 Int64)))
