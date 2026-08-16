(case "ggmin3 the ctor-guard in the ARM but scrutinizing the guard binder differently - guard on tuple-wrapped ctor payload"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap v) (if (> v s) (resume v v) (resume 0 s))))))
                (+ (* 10 (E.rate (Wrap k))) (E.rate (Wrap 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 1 Int64)) (output (: 13 Int64))
  (call   main (: -2 Int64)) (output (: 3 Int64)))
