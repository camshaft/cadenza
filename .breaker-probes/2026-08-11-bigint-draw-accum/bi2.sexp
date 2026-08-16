(case "bi2 recursive draws accumulate into a BIGINT past the Int64 boundary — the limb carry happens in the accumulator, draws stay i64"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (wa (: acc BigInt) (: k Int64))
              (if (< k 1) acc (wa (+ acc (BigInt.of (A.get))) (- k 1))))
            (def (main (: n Int64))
              (handle A 9223372036854775800
                ((get () s (resume s (+ s 1))))
                (let ((want (+ (* (BigInt.of 9223372036854775800) (BigInt.of 2)) (BigInt.of 1))))
                  (if (= (wa (BigInt.of 0) n) want) 1 2))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: 2 Int64)))
