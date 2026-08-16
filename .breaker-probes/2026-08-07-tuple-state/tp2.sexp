(case "tp2 a SWAP op exchanges the tuple-state slots — interleaved readers observe the exchange"
  (input  (do
            (effect Tw (op rd (-> Int64)) (op swap (-> Int64)))
            (def (main (: n Int64))
              (handle Tw (tuple n 100)
                ((rd () s (resume (. s 0) (tuple (+ (. s 0) 1) (. s 1))))
                 (swap () s (resume (. s 1) (tuple (. s 1) (. s 0)))))
                (+ (Tw.rd) (+ (* 10 (Tw.swap)) (+ (Tw.rd) (* 1000 (Tw.swap)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7105 Int64))
  (call   main (: 0 Int64)) (output (: 2100 Int64)))
