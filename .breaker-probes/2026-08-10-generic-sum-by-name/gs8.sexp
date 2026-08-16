(case "gs8 the applied generic AS the handler state — (Container Int64) seeds the handle, the arm unwraps and re-wraps per dispatch"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op tick (-> Int64)))
            (def (main (: k Int64))
              (handle E (Full k)
                ((tick () st (match st
                               ((Full v) (resume v (Full (+ v 3)))))))
                (+ (* 10 (E.tick)) (E.tick))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 47 Int64))
  (call   main (: -1 Int64)) (output (: -8 Int64)))
