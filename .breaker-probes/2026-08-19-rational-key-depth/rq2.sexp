(case "rq2 a Rational-keyed map churned with DIFFERENTLY-NORMALIZED keys equals the direct build"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Rational Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (Rational.of (* i 2) 6) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Rational Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (Rational.of i 3)))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert Map.empty (Rational.of 1 2) 50))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned (Rational.of 2 4)) ((Some v) (if (= v 50) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 11 Int64)))
