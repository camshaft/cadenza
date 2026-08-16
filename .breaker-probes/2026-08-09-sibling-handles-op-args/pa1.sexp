(case "pa1 two SIBLING nested handles as the two arguments of one 2-ary op — each region draws the outer thread in arg order"
  (input  (do
            (effect E (op next (-> Int64)) (op pair (-> Int64 Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (effect C (op h (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (pair (a b) s (resume (+ a (* 100 b)) s)))
                (E.pair
                  (handle B 10 ((g (u) t (resume t t))) (+ (B.g) (E.next)))
                  (handle C 20 ((h (u) t (resume t t))) (+ (C.h) (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2615 Int64))
  (call   main (: 0 Int64)) (output (: 2110 Int64))
  (call   main (: -30 Int64)) (output (: -920 Int64)))
