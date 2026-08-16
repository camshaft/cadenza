(do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (Char.from-int (E.next))
                  ((Some c) (+ (* 10 (Char.to-int c)) 1))
                  ((None _u) 7))))
            (export main))
