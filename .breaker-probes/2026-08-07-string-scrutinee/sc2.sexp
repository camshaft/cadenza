(case "sc2 EQUALITY of two string draws routes the branch — the n=3 row crosses the threshold between draws"
  (input  (do
            (effect St (op name (-> Int64 String)))
            (def (main (: n Int64))
              (handle St n
                ((name (k) s (resume (if (> s k) "big" "sm") (+ s 2))))
                (let ((w1 (St.name 4)))
                  (let ((w2 (St.name 4)))
                    (if (= w1 w2) (String.byte-len (String.concat w1 w2)) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64)))
