(case "sr1 ss-class radius: does the drop hit an observer that is the SAME op (put observing its own prior)"
  (input  (do
            (effect Acc (op put (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume s (+ s 1))))
                (do (def g (grow k)) (Acc.put))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64)))
