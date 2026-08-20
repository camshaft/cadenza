(case "pyrs1 probe: a (Result Int64 Int64) HANDLER STATE threaded across three dispatches — tick answers (Ok v -> v*10 / Err e -> -e) and threads (Ok v -> Ok(v+1) / Err e -> Ok e, recovering on the first tick); seed Err 7 when n%3=0 else Ok(n%3), so the state slot carries a two-arm SUM and the Err arm recovers into Ok mid-thread"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (if (= (% n 3) (: 0 Int64)) (Err (: 7 Int64)) (Ok (% n 3)))
      ((tick () s (resume (match s ((Ok v) (* v 10)) ((Err e) (- (: 0 Int64) e)))
                          (match s ((Ok v) (Ok (+ v 1))) ((Err e) (Ok e))))))
      (+ (* 1000 (E.tick)) (+ (* 100 (E.tick)) (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 12030 Int64))
  (call   main (: 0 Int64)) (output (: 80 Int64)))
