(case "pyr1 POST-RESUME ARITHMETIC in the arm — each tick's arm ADDS a thousandfold toll to whatever the resumed rest-of-body eventually returns, three dispatches stack three tolls that unwind INNERMOST-FIRST after the body's positional fold completes, each toll keyed to the state AT ITS OWN DISPATCH so a reordered unwind or a stale-state toll misprices the pyramid, and the seed shifts every toll together"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 2)) (* 1000 (+ s 1)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (let ((c (E.tick)))
                      (+ a (+ (* 10 b) (* 100 c))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12531 Int64))
  (call   main (: 0 Int64)) (output (: 9420 Int64)))
