(case "oc1 an Option-of-TUPLE accumulator — None seeds on first step, Some carries (total,count) advancing both"
  (input  (do
            (effect O (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (None)
                ((step (v) s (match s
                               ((None) (resume 0 (Some (tuple v 1))))
                               ((Some p) (match p
                                           ((tuple tot cnt) (resume (+ tot cnt)
                                                                    (Some (tuple (+ tot v) (+ cnt 1))))))))))
                (+ (O.step n) (+ (* 10 (O.step 3)) (* 100 (O.step 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1060 Int64))
  (call   main (: 0 Int64)) (output (: 510 Int64)))
