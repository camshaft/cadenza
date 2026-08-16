(case "dh1 value-eq on two ARM-BUILT lists inside a dispatch — the borrowed-handle reclaim after = must not double-drop"
  (input  (do
            (effect Q (op probe (-> Int64 Bool)))
            (def (main (: n Int64))
              (handle Q n
                ((probe (v) s
                  (resume (= (list v (+ v 1)) (list s (+ s 1))) (+ s 1))))
                (+ (if (Q.probe n) 1 0)
                   (+ (* 10 (if (Q.probe n) 1 0))
                      (* 100 (if (Q.probe (+ n 2)) 1 0))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
