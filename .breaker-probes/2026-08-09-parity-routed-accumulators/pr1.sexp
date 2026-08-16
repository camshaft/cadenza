(case "pr1 PARITY routes each tick into one of two accumulator slots — the (a,b,k) state splits the stream by evenness"
  (input  (do
            (effect E (op tick (-> Int64)) (op geta (-> Int64)) (op getb (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 0 n)
                ((tick () s (match s
                              ((tuple a b k)
                                (resume k (if (= (% k 2) 0)
                                              (tuple (+ a k) b (+ k 1))
                                              (tuple a (+ b k) (+ k 1)))))))
                 (geta () s (match s ((tuple a b k) (resume a s))))
                 (getb () s (match s ((tuple a b k) (resume b s)))))
                (do (E.tick) (E.tick) (E.tick) (E.tick)
                    (+ (* 100 (E.geta)) (E.getb)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 608 Int64))
  (call   main (: 1 Int64)) (output (: 604 Int64))
  (call   main (: -3 Int64)) (output (: -204 Int64)))
