(do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)) (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((ask () t (resume (O.next) (+ t (O.next))))
                   (get () t (resume t t)))
                  (+ (* 100 (I.ask)) (+ (* 10 (I.ask)) (I.get))))))
            (export main))
