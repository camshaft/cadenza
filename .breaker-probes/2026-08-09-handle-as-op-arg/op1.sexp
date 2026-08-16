(case "op1 a WHOLE nested handle expression as an op's ARGUMENT beside an outer draw"
  (input  (do
            (effect E (op next (-> Int64)) (op put (-> Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (put (v) s (resume (+ v s) s)))
                (E.put
                  (handle B 100
                    ((g (u) t (resume t (+ t 5))))
                    (+ (B.g) (+ (B.g) (E.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 216 Int64))
  (call   main (: 0 Int64)) (output (: 206 Int64))
  (call   main (: -10 Int64)) (output (: 186 Int64)))
