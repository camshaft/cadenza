(case "slmin5 min3 with a SCALAR string state (no tuple) - puts grow, size-seeded inner"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E "ab"
                ((put () r (resume 0 (String.concat r "x")))
                 (size () r (resume (String.byte-len r) r)))
                (do (E.put) (E.put)
                    (+ (handle B (E.size)
                         ((g (u) t (resume t (+ t 10))))
                         (+ (B.g) (B.g)))
                       (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 22 Int64)))
