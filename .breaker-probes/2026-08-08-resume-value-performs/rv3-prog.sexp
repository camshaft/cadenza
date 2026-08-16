(do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((tick () t (resume t (+ t (O.next)))))
                  (+ (* 100 (I.tick)) (+ (* 10 (I.tick)) (O.next))))))
            (export main))
