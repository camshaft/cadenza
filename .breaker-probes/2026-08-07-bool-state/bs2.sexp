(case "bs2 a LATCH — once an op arg is false, the state stays false and every later check answers false"
  (input  (do
            (effect T (op check (-> Bool Bool)))
            (def (main (: n Int64))
              (handle T true
                ((check (v) s (resume (and s v) (and s v))))
                (+ (if (T.check true) 1 0)
                   (+ (if (T.check (> n 3)) 10 0)
                      (+ (if (T.check true) 100 0) (if (T.check true) 1000 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1111 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
