(case "oc4 an Option state with a RESET cycle — put promotes None to Some or accumulates inside Some; reset reports and returns to None; a fresh put re-promotes"
  (input  (do
            (effect E (op put (-> Int64 Int64)) (op reset (-> Int64)))
            (def (main (: n Int64))
              (handle E (None)
                ((put (v) st (match st
                               ((Some cur) (resume (+ cur v) (Some (+ cur v))))
                               ((None) (resume 0 (Some v)))))
                 (reset () st (match st
                                ((Some cur) (resume cur (None)))
                                ((None) (resume -1 st)))))
                (+ (E.put n)
                   (+ (* 10 (E.put 7))
                      (+ (* 100 (E.reset))
                         (* 1000 (E.put 3)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1320 Int64))
  (call   main (: 0 Int64)) (output (: 770 Int64))
  (call   main (: -2 Int64)) (output (: 550 Int64)))
