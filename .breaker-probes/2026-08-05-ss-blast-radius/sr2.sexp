(case "sr2 radius: recursive grow uses op A, observer is op B — but B is ALSO used inside recursion"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) (Acc.fin) (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s (resume s s)))
                (do (def g (grow k)) g)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
