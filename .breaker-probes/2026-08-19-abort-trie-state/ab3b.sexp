(case "ab3b minimal: abortive arm RETURNS its scalar state"
  (input  (do
            (effect Bail (op out (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Bail n
                ((out (u) s s))
                (+ 1 (Bail.out))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
