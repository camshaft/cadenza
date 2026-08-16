(case "se1 five draws collected into a Set under a cycling state — duplicates collapse, len and membership pin the distinct draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (% (+ s 1) 3))))
                (let ((st (Set.of (list (E.next) (E.next) (E.next) (E.next) (E.next)))))
                  (+ (* 100 (Set.len st))
                     (+ (if (Set.contains st n) 10 0)
                        (if (Set.contains st 5) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 310 Int64))
  (call   main (: 5 Int64)) (output (: 411 Int64))
  (call   main (: 7 Int64)) (output (: 410 Int64)))
