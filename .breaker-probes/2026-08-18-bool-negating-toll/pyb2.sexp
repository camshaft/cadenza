(case "pyb2 THREE PARITY PROBES under a nested and-chain with negating frames — each draw answers its state's evenness and each frame negates the rest-of-body on the way out, the and-chain short-circuits at the FIRST odd state so the seeds stack one or two negations from the same three-probe body, and the final parity of stacked frames decides the branch"
  (input  (do
            (effect E (op probe (-> Bool)))
            (def (main (: n Int64))
              (if (handle E (% n 3)
                    ((probe () s
                      (not (resume (= (% s 2) 0) (+ s 1)))))
                    (and (E.probe) (and (E.probe) (E.probe))))
                  (: 1 Int64) (: 2 Int64)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
