(case "al4 ONE arm-local feeds BOTH slots — the score accumulates into the base while the count rides beside"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 0)
                ((f (v) s (match s
                            ((tuple base count)
                             (let ((score (+ (* v v) base)))
                               (resume (+ score count) (tuple score (+ count 1))))))))
                (+ (E.f 2) (+ (E.f 3) (E.f 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 49 Int64))
  (call   main (: 0 Int64)) (output (: 34 Int64)))
