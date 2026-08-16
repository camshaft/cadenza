(case "dn4 SKIP-LEVEL performs from the innermost region — A's draws cross the B and C frames to the outermost handler twice"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (effect C (op c (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (* s 2))))
                (handle B 10
                  ((b () s (resume s (+ s 1))))
                  (handle C 100
                    ((c () s (resume s (+ s 1))))
                    (+ (A.a) (+ (A.a) (+ (B.b) (C.c))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 125 Int64))
  (call   main (: 1 Int64)) (output (: 113 Int64)))
