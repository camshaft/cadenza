(case "gg6scope same-ctor double arm, ONE dispatch hitting the literal"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap 0) (resume 100 s))
                    ((Wrap v) (resume v s)))))
                (E.rate (Wrap 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64)))
