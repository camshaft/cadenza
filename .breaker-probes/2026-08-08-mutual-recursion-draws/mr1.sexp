(case "mr1 mutual recursion drawing at EVERY level — even levels weight their draw x10, odd levels x1"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64))
              (if (<= k 0) 0 (+ (* 10 (E.next)) (od (- k 1)))))
            (def (od (: k Int64))
              (if (<= k 0) 0 (+ (E.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 4)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 68 Int64))
  (call   main (: 0 Int64)) (output (: 24 Int64))
  (call   main (: -3 Int64)) (output (: -42 Int64)))
