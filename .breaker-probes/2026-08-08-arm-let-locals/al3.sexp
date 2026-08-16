(case "al3 a two-site resume where EACH branch has its own LET local — gap/overshoot named per path, different strides"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (if (> v s)
                              (let ((gap (- v s))) (resume (* gap 10) (+ s 1)))
                              (let ((over (- s v))) (resume over (+ s 2))))))
                (+ (E.f 8) (+ (E.f 3) (E.f 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 43 Int64))
  (call   main (: 0 Int64)) (output (: 170 Int64))
  (call   main (: 10 Int64)) (output (: 16 Int64)))
