(case "slmin9 growing put WITHOUT the branch (constant yz suffix), two sizes"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1) (String.concat r "yz"))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
