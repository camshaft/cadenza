(case "xh1 TODO-FLIP: a nested-handler op whose ARG is an OUTER handler's op-result (v-effects sweep find)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 0
                  ((put (v) s (resume (+ s v) (+ s v))))
                  (B.put (A.get)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))
