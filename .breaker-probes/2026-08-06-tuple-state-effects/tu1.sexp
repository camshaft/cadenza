(case "tu1 a TUPLE handler state — the arm reads one slot and rebuilds both (pair accumulator)"
  (input  (do
            (effect St (op step (-> Int64 Int64)) (op sum (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (tuple 0 100)
                ((step (v) s
                  (match s
                    ((tuple lo hi)
                      (if (> v 10)
                        (resume (+ v hi) (tuple lo (+ hi 1)))
                        (resume lo (tuple (+ lo v) hi))))))
                 (sum (u) s (match s ((tuple lo hi) (resume (+ lo hi) s)))))
                (+ (St.step 20) (+ (St.step n) (* 1000 (St.sum))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 104120 Int64)))
