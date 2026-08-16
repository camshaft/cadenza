(case "gg5scope same-ctor DOUBLE arm without any guard (literal payload then general) x 2 dispatches"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap 0) (resume 100 s))
                    ((Wrap v) (resume v s)))))
                (+ (* 10 (E.rate (Wrap k))) (E.rate (Wrap 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64))
  (call   main (: 0 Int64)) (output (: 1100 Int64)))
