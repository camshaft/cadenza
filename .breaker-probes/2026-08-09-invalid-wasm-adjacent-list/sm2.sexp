(case "sm2 ADJACENCY-PROBE: the branch merges CONCAT-vs-UNCHANGED (one arm grows, one passes through) — is two-concats required"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1)
                                                (if (= (% s 3) 0) (String.concat r "x") r))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 6 Int64))
  (call   main (: 1 Int64)) (output (: 4 Int64))
  (call   main (: -2 Int64)) (output (: 4 Int64)))
