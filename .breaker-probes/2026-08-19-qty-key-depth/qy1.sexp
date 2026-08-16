(case "qy1 a trie of QUANTITY keys resolves a cross-normalized magnitude lookup"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Qty.of (Rational.of i 2) (Unit.base #"meter")) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Qty.of (Rational.of 10 4) (Unit.base #"meter"))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 305 Int64)))
