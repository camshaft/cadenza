(case "rq1 a trie of RATIONAL keys with mixed denominators enumerates in numeric order"
  (input  (do
            (def (fill (: i Int64) (: m (Map Rational Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Rational.of i (+ i 1)) i))))
            (def (inc (: ps (List (Tuple Rational Int64))) (: prev Rational) (: cnt Int64))
              (match ps
                ((list) cnt)
                ((list h .. t) (match h ((tuple k _v) (if (< prev k) (inc t k (+ cnt 1)) -100000))))))
            (def (main (: n Int64))
              (inc (Map.to-list (fill n Map.empty)) (Rational.of 0 1) 0))
            (export main)))
  (call   main (: 40 Int64)) (output (: 40 Int64)))
