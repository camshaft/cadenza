(case "dbn1 a DEBOUNCE gate — emits fire only when the gap since the last fire is met, suppressed emits stash their value in the pending slot, and flush releases whatever the last suppression left"
  (input  (do
            (effect S
              (op emit (-> Int64 Int64 Int64))
              (op flush (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple -100 0)
                ((emit (t v) st
                  (match st
                    ((tuple last pending)
                      (if (>= (- t last) 10)
                          (resume v (tuple t 0))
                          (resume 0 (tuple last v))))))
                 (flush () st
                  (match st
                    ((tuple last pending) (resume pending (tuple last 0))))))
                (let ((a (S.emit 0 7)))
                  (let ((b (S.emit n 8)))
                    (let ((c (S.emit 15 9)))
                      (let ((d (S.flush)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7000900 Int64))
  (call   main (: 12 Int64)) (output (: 7080009 Int64)))
