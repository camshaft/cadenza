(case "ab2 a conditional abort under a MAP-state outer — the pre-abort insert is committed either way"
  (input  (do
            (effect R (op touch (-> Int64 Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle R (map (1 10))
                   ((touch (k) s (resume (Map.len s) (Map.insert s k k))))
                   (+ (R.touch 5)
                      (handle Bail 0
                        ((bail (v) s v))
                        (let ((g (if (> n 3) (Bail.bail 77) 0)))
                          (+ g (R.touch 6))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5078 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
