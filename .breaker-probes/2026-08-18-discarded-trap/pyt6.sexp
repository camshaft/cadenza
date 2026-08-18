(case "pyt6 the WOULD-TRAP REPLAY IS DISCARD-ELIDED — the double-replaying arm's first replay runs a body whose division would trap on the zero seed, but the replay's value is discarded so the pure trapping tail is elided per the discard law and only the second replay's quotient survives, while the nonzero seed shows the first replay otherwise runs"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (+ s 2)))))
                (let ((a (E.tick)))
                  (/ 60 a))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))
