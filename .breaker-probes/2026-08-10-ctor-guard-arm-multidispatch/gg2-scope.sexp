(case "gg2scope multi-variant ctor guard in arm x 2 dispatches"
  (input  (do
            (type (Either a b) (Left a) (Right b))
            (effect E (op rate (-> (Either Int64 Int64) Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Left v) (> v s)) (resume v v))
                    (_other (resume 9 s)))))
                (+ (* 10 (E.rate (Left k))) (E.rate (Right 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 59 Int64))
  (call   main (: -1 Int64)) (output (: 99 Int64)))
