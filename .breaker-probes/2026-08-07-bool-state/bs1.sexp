(case "bs1 a BOOL toggle state — four draws alternate true/false from an input-dependent seed"
  (input  (do
            (effect T (op flip (-> Bool)))
            (def (main (: n Int64))
              (handle T (> n 3)
                ((flip () s (resume s (not s))))
                (+ (if (T.flip) 1 0)
                   (+ (if (T.flip) 10 0)
                      (+ (if (T.flip) 100 0) (if (T.flip) 1000 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 1010 Int64)))
