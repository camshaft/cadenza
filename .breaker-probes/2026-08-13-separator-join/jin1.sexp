(case "jin1 a SEPARATOR-JOIN builder — the comma inserts only BETWEEN elements via the count field, and an EMPTY piece still consumes a separator slot"
  (input  (do
            (effect S (op add (-> String Int64)))
            (def (main (: n Int64))
              (handle S (tuple "" 0)
                ((add (p) st
                  (match st
                    ((tuple s cnt)
                      (let ((s2 (String.concat (if (> cnt 0) (String.concat s ",") s) p)))
                        (resume (String.byte-len s2) (tuple s2 (+ cnt 1))))))))
                (let ((a (S.add "ab")))
                  (let ((b (S.add (if (= n 0) "" "x"))))
                    (let ((c (S.add "cd")))
                      (+ (* 100 (+ (* 100 a) b)) c))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 20306 Int64))
  (call   main (: 1 Int64)) (output (: 20407 Int64)))
