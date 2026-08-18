(case "pyd1 DOUBLE REPLAY UNDER A POST-RESUME TOLL — the arm replays the tail twice inside a do then adds a thousandfold toll to the do's value, the toll fires ONCE per dispatch (not once per replay) on the second replay's outcome, and the seed shifts the surviving replay value and the toll together"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (do (resume s (+ s 1))
                         (resume (+ s 10) (+ s 2)))
                     (* 1000 (+ s 1)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2011 Int64))
  (call   main (: 0 Int64)) (output (: 1010 Int64)))
