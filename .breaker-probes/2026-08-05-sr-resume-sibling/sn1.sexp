(case "sn1 does the sr silent-drop have a RESUMING sibling? recursive advances observed by a resuming op whose RESULT is checked"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s (resume (* 100 s) s)))
                (do (def _g (grow k)) (Acc.fin))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 300 Int64)))
