(case "gs3 the applied generic sum CROSSES a dispatch — (Container Int64) as an op result, unwrapped in the body after the arm wraps the state"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op box (-> (Container Int64))))
            (def (main (: k Int64))
              (handle E k
                ((box () s (resume (Full s) (+ s 1))))
                (+ (match (E.box) ((Full v) v))
                   (* 10 (match (E.box) ((Full v) v))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 87 Int64))
  (call   main (: -4 Int64)) (output (: -34 Int64)))
