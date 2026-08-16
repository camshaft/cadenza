(case "slmin4 min3 but the puts DON'T grow the string (constant suffix dropped - state still tuple)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r) (resume s (tuple (+ s 1) r)))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put)
                    (+ (handle B (E.size)
                         ((g (u) t (resume t (+ t 10))))
                         (+ (B.g) (B.g)))
                       (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 16 Int64)))
