(case "uv5 @ensures on BOTH halves of a performing mutual pair — the postcondition re-checks at every alternating return"
  (input  (do
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
            (export main)))
  (call   main (: 2 Int64)) (output (: 71 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64)))
