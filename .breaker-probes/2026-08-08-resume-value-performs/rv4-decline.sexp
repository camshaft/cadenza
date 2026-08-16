(case "rv4 the arm draws from the outer handler in BOTH the resume value AND the next-state expression — two outer advances per inner dispatch"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)) (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((ask () t (resume (O.next) (+ t (O.next))))
                   (get () t (resume t t)))
                  (+ (* 100 (I.ask)) (+ (* 10 (I.ask)) (I.get))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 248 Int64))
  (call   main (: 0 Int64)) (output (: 24 Int64)))
