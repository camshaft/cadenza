(case "ss3 a string state grows CONDITIONALLY — even ticks append, odd ticks pass through unchanged"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple "" n)
                ((tick () s (match s
                              ((tuple acc k)
                                (resume (String.byte-len acc)
                                        (tuple (if (= (% k 2) 0) (String.concat acc "ab") acc)
                                               (+ k 1)))))))
                (do (E.tick) (E.tick) (E.tick)
                    (+ (* 10 (E.tick)) 3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 43 Int64))
  (call   main (: 1 Int64)) (output (: 23 Int64)))
