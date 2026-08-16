(case "ggmin2 the same ctor-guard in BODY position (no handler)"
  (input  (do
            (type Box (Wrap Int64))
            (def (rate (: c Box) (: s Int64))
              (match c
                ((guard (Wrap v) (> v s)) v)
                ((Wrap _v) 0)))
            (def (main (: k Int64))
              (+ (* 10 (rate (Wrap k) 0)) (rate (Wrap 3) 100)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
