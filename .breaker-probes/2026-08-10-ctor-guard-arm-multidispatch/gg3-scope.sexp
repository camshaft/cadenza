(case "gg3scope single-variant ctor guard with WILDCARD fallback (not same-ctor)"
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Wrap v) (> v s)) (resume v v))
                    (_other (resume 0 s)))))
                (+ (* 10 (E.rate (Wrap k))) (E.rate (Wrap 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
