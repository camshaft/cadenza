(case "sm1 ADJACENCY-PROBE: the phi-merge at the SEED (branch-picked initial rope), growth constant — is the merge-in-arm the trigger or any rope phi"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (if (= (% n 3) 0) "x" "yz"))
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1) (String.concat r "a"))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 6 Int64))
  (call   main (: 1 Int64)) (output (: 8 Int64))
  (call   main (: -2 Int64)) (output (: 8 Int64)))
