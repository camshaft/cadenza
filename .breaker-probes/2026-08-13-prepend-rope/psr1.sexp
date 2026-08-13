(case "psr1 a PREPEND-rope accumulator — each push prepends its piece so the OLDEST piece rides at the END, the closing slice proves the reversal order and the piece width varies by seed"
  (input  (do
            (effect S
              (op push (-> String Int64))
              (op check (-> String Int64)))
            (def (main (: n Int64))
              (handle S ""
                ((push (p) s
                  (let ((s2 (String.concat p s)))
                    (resume (String.byte-len s2) s2)))
                 (check (p) s
                  (resume (match (String.slice s (- (String.byte-len s) (String.byte-len p)) (String.byte-len s))
                            ((Some w) (if (= w p) 1 0))
                            ((None u) -1))
                          s)))
                (let ((p1 (if (= n 0) "x" "xyz")))
                  (let ((a (S.push p1)))
                    (let ((b (S.push "cd")))
                      (let ((c (S.push "ef")))
                        (let ((ok (S.check p1)))
                          (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) ok))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 13051 Int64))
  (call   main (: 1 Int64)) (output (: 35071 Int64)))
