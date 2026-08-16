(case "gg4scope multi-variant with SAME-CTOR fallback ((Left _v) not wildcard)"
  (input  (do
            (type (Either a b) (Left a) (Right b))
            (effect E (op rate (-> (Either Int64 Int64) Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Left v) (> v s)) (resume v v))
                    ((Left _v) (resume 0 s))
                    ((Right _w) (resume 9 s)))))
                (+ (* 10 (E.rate (Left k))) (E.rate (Left 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
