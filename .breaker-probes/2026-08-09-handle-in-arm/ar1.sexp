(case "ar1 a WHOLE nested handle expression inside a handler ARM's resume value"
  (input  (do
            (effect E (op boost (-> Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((boost (v) s
                  (resume
                    (handle B (+ s v)
                      ((g (u) t (resume t (+ t 3))))
                      (+ (B.g) (B.g)))
                    (+ s 1))))
                (+ (E.boost 10) (E.boost 20))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64))
  (call   main (: 0 Int64)) (output (: 68 Int64))
  (call   main (: -17 Int64)) (output (: 0 Int64)))
