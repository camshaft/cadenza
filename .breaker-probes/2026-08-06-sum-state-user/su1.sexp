(case "su1 a USER sum as handler state — a countdown mode machine (Fast k -> Slow)"
  (input  (do
            (effect St (op step (-> Unit Int64)))
            (type Mode (Fast Int64) (Slow))
            (def (main (: n Int64))
              (handle St (Mode.Fast n)
                ((step (u) s
                  (match s
                    ((Mode.Fast k) (if (> k 0) (resume k (Mode.Fast (- k 1))) (resume 0 (Mode.Slow))))
                    ((Mode.Slow) (resume -1 (Mode.Slow))))))
                (+ (* 100 (St.step)) (+ (* 10 (St.step)) (St.step)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 210 Int64)))
