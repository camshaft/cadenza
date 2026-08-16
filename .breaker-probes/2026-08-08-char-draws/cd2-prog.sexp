(do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((p (% (E.next) 2)))
                  (let ((c (if (= p 0) #\a #\z)))
                    (+ (* 10 (Char.to-int c)) p)))))
            (export main))
