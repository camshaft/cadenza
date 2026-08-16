(case "ac3 the pipeline BOUNCES between two effects — inner B's result feeds outer E's op, whose result feeds B again"
  (input  (do
            (effect E (op inc (-> Int64 Int64)))
            (effect B (op g (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1))))
                (handle B 100
                  ((g (x) t (resume (+ x t) (+ t 5))))
                  (B.g (E.inc (B.g 3))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 213 Int64))
  (call   main (: 0 Int64)) (output (: 208 Int64))
  (call   main (: -4 Int64)) (output (: 204 Int64)))
