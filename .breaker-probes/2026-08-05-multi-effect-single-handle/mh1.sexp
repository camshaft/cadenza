(case "mh1 alternating deep interleave: A-B-A-B-A-B six performs with mutually-referencing resume values"
  (input  (do
            (effect A (op a (-> Int64 Int64)))
            (effect B (op b (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a (v) s (resume (+ v s) (+ s 1))))
                (handle B 100
                  ((b (v) t (resume (+ v t) (+ t 10))))
                  (B.b (A.a (B.b (A.a (B.b (A.a 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 348 Int64)))
