(case "ggmin4 single dispatch, ctor guard in arm, should admit"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Wrap v) (> v s)) (resume v v))
                    ((Wrap _v) (resume 0 s)))))
                (E.rate (Wrap k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
