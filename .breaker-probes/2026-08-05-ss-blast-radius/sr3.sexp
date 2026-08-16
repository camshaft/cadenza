(case "sr3 radius: the untouched-op observer AFTER recursion (ss2f shape) with the ARM resuming state not 0"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume s (+ s 1)))
                 (fin (u) s (resume s s)))
                (do (def g (grow k)) (+ (* 10 g) (Acc.fin)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 12 Int64)))
