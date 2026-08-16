(case "sb3 the op ARG draws from effect B while the dispatch homes to effect A — two paired dispatches, both states advance"
  (input  (do
            (effect A (op add (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((add (v) s (resume (+ v s) (+ s 1))))
                (handle B n
                  ((next () t (resume t (* t 2))))
                  (+ (A.add (B.next)) (A.add (B.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 36 Int64))
  (call   main (: 0 Int64)) (output (: 21 Int64)))
