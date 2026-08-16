(case "cd2 draw parity picks WHICH constant char flows to Char.to-int — the char value crosses the branch join"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((p (% (E.next) 2)))
                  (let ((c (if (= p 0) #\a #\z)))
                    (+ (* 10 (Char.to-int c)) p)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 970 Int64))
  (call   main (: 7 Int64)) (output (: 1221 Int64)))
