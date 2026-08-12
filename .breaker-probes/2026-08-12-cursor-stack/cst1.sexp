(case "cst1 a CURSOR-STACK state — the tuple (buf,top) pushes by List.update-or-append at the cursor and pops by decrement, stale slots above the cursor are overwritten, the over-pop answers a sentinel"
  (input  (do
            (effect S
              (op push (-> Int64 Int64))
              (op pop (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (: (list) (List Int64)) 0)
                ((push (v) st
                  (match st
                    ((tuple buf top)
                      (let ((buf2 (if (< top (List.len buf))
                                      (List.update buf top v)
                                      (List.push buf v))))
                        (resume (+ top 1) (tuple buf2 (+ top 1)))))))
                 (pop () st
                  (match st
                    ((tuple buf top)
                      (if (= top 0)
                          (resume -1 st)
                          (resume (match (List.at buf (- top 1)) ((Some x) x) ((None u) -99))
                                  (tuple buf (- top 1))))))))
                (let ((a (S.push n)))
                  (let ((b (S.push (+ n 1))))
                    (let ((c (S.pop)))
                      (let ((d (S.push 50)))
                        (let ((e (S.pop)))
                          (let ((f (S.pop)))
                            (let ((g (S.pop)))
                              (+ (* 10 (+ (* 100 (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) d)) e)) f)) (+ g 2)))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1204250031 Int64))
  (call   main (: 6 Int64)) (output (: 1207250061 Int64)))
