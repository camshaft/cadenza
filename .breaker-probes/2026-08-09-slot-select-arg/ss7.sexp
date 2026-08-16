(case "ss7 a 2-ary op carries SELECTOR and MAGNITUDE — which slot and by how much are both payload-driven, a trailing read pins slot 0"
  (input  (do
            (effect E (op sel (-> Int64 Int64 Int64)) (op rd0 (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((sel (i d) st (match st
                                 ((tuple a b c)
                                  (if (= i 0) (resume a (tuple (+ a d) b c))
                                      (if (= i 1) (resume b (tuple a (+ b d) c))
                                          (resume c (tuple a b (+ c d))))))))
                 (rd0 () st (match st ((tuple a b c) (resume a st)))))
                (+ (E.sel 0 3) (+ (E.sel 2 7) (E.rd0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 113 Int64))
  (call   main (: 0 Int64)) (output (: 103 Int64)))
