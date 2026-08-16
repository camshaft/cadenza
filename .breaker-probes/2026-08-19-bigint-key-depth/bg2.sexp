(case "bg2 a BigInt-keyed map churned back keys and lookups like the direct build across limbs"
  (input  (do
            (def big (BigInt.of 9223372036854775807))
            (def (grow (: i Int64) (: n Int64) (: m (Map BigInt Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (* big (BigInt.of (+ i 10))) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map BigInt Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (* big (BigInt.of (+ i 10)))))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert (Map.insert Map.empty (* big (BigInt.of 2)) 20) (BigInt.of 5) 50))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned (* big (BigInt.of 2))) ((Some v) (if (= v 20) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 11 Int64)))
