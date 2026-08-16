(case "qt2 a BigInt handler state through a two-site arm (heap-scalar state face)"
  (input  (do
            (effect Acc (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle Acc (BigInt.of a)
                ((feed (v) s (if (> v 10) (resume (+ v (Int64.of s)) (+ s 1N)) (resume 0 s))))
                (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2536 Int64)))
