(case "gs7 the applied generic in the op ARGUMENT position — the arm unwraps (Container Int64) payloads into the accumulating state"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op feed (-> (Container Int64) Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((feed (c) s (match c
                               ((Full v) (resume (+ s v) (+ s v))))))
                (+ (* 10 (E.feed (Full k))) (E.feed (Full 5)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 38 Int64))
  (call   main (: -4 Int64)) (output (: -39 Int64)))
