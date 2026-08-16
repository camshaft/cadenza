(case "slmin3 min2 + the two put calls back (full sl1 minus nothing else)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1)
                                                (String.concat r (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put)
                    (+ (handle B (E.size)
                         ((g (u) t (resume t (+ t 10))))
                         (+ (B.g) (B.g)))
                       (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 25 Int64)))
