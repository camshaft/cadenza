(case "bg1 a trie of BigInt keys SPANNING the limb boundary enumerates in numeric order"
  (input  (do
            (def (fill (: i Int64) (: m (Map BigInt Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (* (BigInt.of 9223372036854775807) (BigInt.of i)) i))))
            (def (inc (: ps (List (Tuple BigInt Int64))) (: prev BigInt) (: cnt Int64))
              (match ps
                ((list) cnt)
                ((list h .. t) (match h ((tuple k _v) (if (< prev k) (inc t k (+ cnt 1)) -100000))))))
            (def (main (: n Int64))
              (inc (Map.to-list (fill n Map.empty)) (BigInt.of 0) 0))
            (export main)))
  (call   main (: 40 Int64)) (output (: 40 Int64)))
