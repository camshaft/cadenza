(case "pyos1 probe: an (Option Int64) HANDLER STATE threaded across three dispatches — tick answers (Some v -> v*10 / None -> -1) and threads (Some v -> Some(v+1) / None -> Some 0); the seed is None when n%3=0 else Some(n%3), so the state slot carries a two-arm SUM VALUE and a match reads/rebuilds it per dispatch as it threads"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (if (= (% n 3) (: 0 Int64)) (None) (Some (% n 3)))
      ((tick () s (resume (match s ((Some v) (* v 10)) ((None) (: -1 Int64)))
                          (match s ((Some v) (Some (+ v 1))) ((None) (Some (: 0 Int64)))))))
      (+ (* 1000 (E.tick)) (+ (* 100 (E.tick)) (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 12030 Int64))
  (call   main (: 0 Int64)) (output (: -990 Int64)))
