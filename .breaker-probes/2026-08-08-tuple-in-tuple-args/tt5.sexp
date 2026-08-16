(case "tt5 an inner PAIR swapped on counter parity — a nested-tuple state machine, both slots and the counter observed"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (tuple n 100) 0)
                ((tick () s (match s
                              ((tuple pr c) (match pr
                                              ((tuple x y) (resume (+ x c)
                                                                   (if (= (% c 2) 0)
                                                                       (tuple (tuple y x) (+ c 1))
                                                                       (tuple (tuple x y) (+ c 1))))))))))
                (+ (E.tick) (+ (* 10 (E.tick)) (+ (* 100 (E.tick)) (* 1000 (E.tick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 19215 Int64))
  (call   main (: 0 Int64)) (output (: 14210 Int64)))
