(do
            (effect E (op next (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (let ((d (E.next)))
                    (if (> d 7)
                        (Bail.out d)
                        (+ d (walk (- k 1)))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out (v) t v))
                            (walk 5)))
                   (E.next))))
            (export main))
