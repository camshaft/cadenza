(case "id1 a state-STAMPED identity chained three deep — each hop adds the live state to the passing value"
  (input  (do
            (effect E (op stamp (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((stamp (x) s (resume (+ x s) (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (E.stamp (E.stamp (E.stamp 7))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 163 Int64))
  (call   main (: 0 Int64)) (output (: 103 Int64))
  (call   main (: -4 Int64)) (output (: -17 Int64)))
