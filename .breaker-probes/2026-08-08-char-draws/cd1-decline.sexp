(case "cd1 a DRAW is the Char.from-int code point — validity gates the branch, to-int round-trips the accepted draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (Char.from-int (E.next))
                  ((Some c) (+ (* 10 (Char.to-int c)) 1))
                  ((None _u) 7))))
            (export main)))
  (call   main (: 97 Int64)) (output (: 971 Int64))
  (call   main (: 55296 Int64)) (output (: 7 Int64))
  (call   main (: 57344 Int64)) (output (: 573441 Int64)))
