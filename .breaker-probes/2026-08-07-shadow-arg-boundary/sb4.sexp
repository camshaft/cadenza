(case "sb4 the innermost seed is a COMPOSITE of two effects' dispatches — B's draw feeds A's add, the result seeds a fresh B shadow"
  (input  (do
            (effect A (op add (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((add (v) s (resume (+ v s) (+ s 1))))
                (handle B n
                  ((next () t (resume t (* t 2))))
                  (handle B (A.add (B.next))
                    ((next () t (resume t (- t 3))))
                    (+ (B.next) (B.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 27 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64)))
