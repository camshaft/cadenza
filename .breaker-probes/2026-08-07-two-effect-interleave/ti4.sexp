(case "ti4 THREE-effect chained cross-feed — C's arm performs B, B's arm performs A, two C dispatches walk the whole chain twice"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (effect C (op c (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 100
                  ((b () t (resume (+ t (A.a)) (+ t 1))))
                  (handle C 10000
                    ((c () u (resume (+ u (B.b)) (+ u 1))))
                    (+ (C.c) (C.c))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20213 Int64))
  (call   main (: 0 Int64)) (output (: 20203 Int64)))
