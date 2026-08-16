(case "sr2 a Result-typed op whose Err carries a STRING reason — the body matches Ok payload vs the reason by string equality"
  (input  (do
            (type Res (Ok Int64) (Err String))
            (effect E (op step (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((step () s
                  (resume (if (= (% s 2) 0) (Res.Ok s) (Res.Err "odd")) (+ s 3))))
                (let ((score (fn ((: r Res))
                               (match r
                                 ((Res.Ok v) v)
                                 ((Res.Err why) (if (= why "odd") 7 1))))))
                  (+ (* 10 (score (E.step))) (score (E.step))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 27 Int64))
  (call   main (: 1 Int64)) (output (: 74 Int64))
  (call   main (: -4 Int64)) (output (: -33 Int64)))
