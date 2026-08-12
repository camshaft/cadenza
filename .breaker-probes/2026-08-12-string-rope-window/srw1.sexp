(case "srw1 a GROWING STRING state with a computed slice window per dispatch — each grow appends two chars, the arm answers the byte-len of an interior window built from the drawn offset"
  (input  (do
            (effect S (op grow (-> String Int64 Int64)))
            (def (main (: n Int64))
              (handle S "ab"
                ((grow (add lo) s
                  (let ((s2 (String.concat s add)))
                    (resume (match (String.slice s2 lo (- (String.byte-len s2) 1))
                              ((Some w) (String.byte-len w))
                              ((None u) -1))
                            s2))))
                (let ((a (S.grow "cd" (+ n 1))))
                  (let ((b (S.grow "ef" (+ n 2))))
                    (+ (* 100 a) (* 10 b))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 230 Int64))
  (call   main (: 1 Int64)) (output (: 120 Int64)))
