(case "scc1 a HIGH-WATER STRING state — the arm compares each drawn string lexicographically against the champion, keeps the max, and the final read exposes the winner's length"
  (input  (do
            (effect S
              (op put (-> String Int64))
              (op len (-> Int64)))
            (def (main (: n Int64))
              (handle S ""
                ((put (s) hw
                  (if (< hw s)
                      (resume 1 s)
                      (resume 0 hw)))
                 (len () hw (resume (String.byte-len hw) hw)))
                (let ((a (S.put "banana")))
                  (let ((b (S.put "apple")))
                    (let ((c (S.put (String.concat "cherry" (if (> n 0) "x" "")))))
                      (let ((d (S.len)))
                        (+ (* 100 (+ (* 10 (+ (* 10 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10106 Int64))
  (call   main (: 1 Int64)) (output (: 10107 Int64)))
