(case "lx1 LEXICOGRAPHIC order on the rope state — the growing string crosses the 'mm' threshold exactly at the second push"
  (input  (do
            (effect S (op push (-> Int64)) (op past (-> Int64)))
            (def (main (: n Int64))
              (handle S ""
                ((push () s (resume (String.byte-len s) (String.concat s "m")))
                 (past () s (resume (if (< s "mm") 0 1) s)))
                (let ((_a (S.push)))
                  (let ((p1 (S.past)))
                    (let ((_b (S.push)))
                      (let ((p2 (S.past)))
                        (let ((_c (S.push)))
                          (let ((p3 (S.past)))
                            (+ (* 100 p1) (+ (* 10 p2) p3))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
