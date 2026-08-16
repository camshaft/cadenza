(case "gg7scope THREE same-ctor arms (two literals + general) x 3 dispatches"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap 0) (resume 100 s))
                    ((Wrap 1) (resume 200 s))
                    ((Wrap v) (resume v s)))))
                (+ (E.rate (Wrap k))
                   (+ (* 10 (E.rate (Wrap 0)))
                      (* 100 (E.rate (Wrap 1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21005 Int64))
  (call   main (: -3 Int64)) (output (: 20997 Int64)))
