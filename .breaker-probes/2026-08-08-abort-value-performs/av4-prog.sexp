(do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)) (op mark (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out () t (Bail.mark))
                             (mark () t (resume t (+ t 3))))
                            (+ (Bail.out) 999)))
                   (O.next))))
            (export main))
