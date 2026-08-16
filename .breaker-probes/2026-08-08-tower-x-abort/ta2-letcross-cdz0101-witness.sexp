(do
            (effect E (op next (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k (E.next)))
                  (+ (* 10 (handle E (* 10 k)
                             ((next () s (resume s (+ s 5))))
                             (+ (handle Bail 0
                                  ((out (v) t (+ 1000 v)))
                                  (let ((d (E.next)))
                                    (if (> d 52) (Bail.out d) d)))
                                (E.next))))
                     (- (E.next) n)))))
            (export main))
