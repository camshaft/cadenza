(case "gcd1 EUCLID with a logged trace — each remainder step performs, the accumulator sums the divisor chain, data-dependent step counts"
  (input  (do
            (effect S (op log (-> Int64 Int64)) (op sum (-> Int64)))
            (def (gcd (: a Int64) (: b Int64) (: k Int64))
              (if (< k 1) a
                (if (= b 0) a
                  (let ((_l (S.log b)))
                    (gcd b (% a b) (- k 1))))))
            (def (main (: n Int64))
              (handle S 0
                ((log (v) acc (resume v (+ acc v)))
                 (sum () acc (resume acc acc)))
                (let ((g (gcd n 12 20)))
                  (+ (* 1000 g) (S.sum)))))
            (export main)))
  (call   main (: 18 Int64)) (output (: 6018 Int64))
  (call   main (: 35 Int64)) (output (: 1024 Int64))
  (call   main (: 12 Int64)) (output (: 12012 Int64)))
