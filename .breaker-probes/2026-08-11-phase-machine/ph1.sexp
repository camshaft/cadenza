(case "ph1 a THREE-phase state machine — Idle to Running(count,peak) to Done(total), the sentinel input drives the final transition"
  (input  (do
            (type Phase (Idle) (Running Int64 Int64) (Done Int64))
            (effect M (op step (-> Int64 Int64)) (op query (-> Int64)))
            (def (main (: n Int64))
              (handle M (Idle)
                ((step (v) st
                  (match st
                    ((Idle) (resume 0 (Running v v)))
                    ((Running c p) (resume c (if (< v 0) (Done (+ c p)) (Running (+ c v) (if (> v p) v p)))))
                    ((Done t) (resume t st))))
                 (query () st
                  (resume (match st ((Idle) -1) ((Running c p) (+ (* 100 c) p)) ((Done t) (+ 10000 t))) st)))
                (let ((_a (M.step n)))
                  (let ((_b (M.step 7)))
                    (let ((_c (M.step -1)))
                      (M.query))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10017 Int64))
  (call   main (: 9 Int64)) (output (: 10025 Int64)))
