(case "sc2 ADJACENCY-PROBE: SCALAR string state (no tuple) with the two-fresh-concats phi in the arm — is the tuple projection required"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E "ab"
                ((put () r (resume 0 (if (= (% (String.byte-len r) 2) 0)
                                         (String.concat r "x")
                                         (String.concat r "yz"))))
                 (size () r (resume (String.byte-len r) r)))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: 10 Int64)))
