(case "gsx1 ONE arm instantiates the same generic sum at TWO types — a Container Int64 and a Container String built, unwrapped, and combined per dispatch"
  (input  (do
            (type (Container a) (Full a))
            (effect S (op mix (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((mix () s
                  (let ((ci (: (Full s) (Container Int64))))
                    (let ((cs (: (Full "abc") (Container String))))
                      (resume (+ (* 10 (match ci ((Full v) v)))
                                 (match cs ((Full w) (String.byte-len w))))
                              (+ s 1))))))
                (let ((a (S.mix)))
                  (let ((b (S.mix)))
                    (+ (* 1000 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33043 Int64))
  (call   main (: 20 Int64)) (output (: 203213 Int64)))
