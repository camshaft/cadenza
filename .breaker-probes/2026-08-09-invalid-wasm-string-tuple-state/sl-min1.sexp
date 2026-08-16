(case "slmin1 heap tuple state + nested handle seeded by an op result"
  (input  (do
            (effect E (op size (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (+ (handle B (E.size)
                     ((g (u) t (resume t (+ t 10))))
                     (+ (B.g) (B.g)))
                   (E.size))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 16 Int64)))
