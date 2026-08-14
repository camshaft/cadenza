(case "tth1 TORTOISE-AND-HARE cycle detection — the handler owns the successor function and counts calls, the body's two-speed recursive driver meets inside the cycle and the call tally rides in the answer"
  (input  (do
            (effect S
              (op succ (-> Int64 Int64))
              (op calls (-> Int64)))
            (def (chase (: slow Int64) (: fast Int64) (: k Int64) (: steps Int64))
              (if (< k 1)
                  (* steps 1000)
                  (let ((s2 (S.succ slow)))
                    (let ((f2 (S.succ (S.succ fast))))
                      (if (= s2 f2)
                          (+ (* (+ steps 1) 1000) (* s2 10))
                          (chase s2 f2 (- k 1) (+ steps 1)))))))
            (def (main (: n Int64))
              (handle S 0
                ((succ (i) c (resume (% (+ (* i 2) n) 6) (+ c 1)))
                 (calls () c (resume (% c 10) c)))
                (let ((r (chase 0 0 8 0)))
                  (+ r (S.calls)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2036 Int64))
  (call   main (: 2 Int64)) (output (: 2006 Int64)))
