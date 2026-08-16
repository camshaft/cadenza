(case "oc6 an Option-of-TUPLE state — the Some payload is a (total,count) pair updated element-wise per dispatch, a final read reports both"
  (input  (do
            (effect E (op mark (-> Int64 Int64)) (op report (-> Int64)))
            (def (main (: n Int64))
              (handle E (None)
                ((mark (v) st (match st
                                ((Some p) (match p
                                            ((tuple a c) (resume (+ a v) (Some (tuple (+ a v) (+ c 1)))))))
                                ((None) (resume 0 (Some (tuple v 1))))))
                 (report () st (match st
                                 ((Some p) (match p ((tuple a c) (resume (+ (* 1000 c) a) st))))
                                 ((None) (resume -1 st)))))
                (+ (E.mark n)
                   (+ (* 10 (E.mark 4))
                      (+ (* 100 (E.mark -2))
                         (E.report))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3797 Int64))
  (call   main (: 0 Int64)) (output (: 3242 Int64))
  (call   main (: -7 Int64)) (output (: 2465 Int64)))
