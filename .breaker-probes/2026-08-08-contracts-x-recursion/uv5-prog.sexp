(do
            (effect E (op next (-> Int64)))
            (@ (ensures (>= ret 0))
               (def (ev (: k Int64) (: acc Int64))
                 (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next))))))
            (@ (ensures (>= ret 0))
               (def (od (: k Int64) (: acc Int64))
                 (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 4 0)))
            (export main))
