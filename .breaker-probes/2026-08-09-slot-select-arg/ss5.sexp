(case "ss5 an op ARG selects WHICH tuple-state slot to bump — index-routed slot mutation, four dispatches revisit slot 0"
  (input  (do
            (effect E (op sel (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((sel (i) st (match st
                               ((tuple a b c)
                                (if (= i 0) (resume a (tuple (+ a 1) b c))
                                    (if (= i 1) (resume b (tuple a (+ b 1) c))
                                        (resume c (tuple a b (+ c 1)))))))))
                (+ (E.sel 0) (+ (E.sel 2) (+ (E.sel 1) (E.sel 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
