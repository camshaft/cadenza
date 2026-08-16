(case "st2 draws BETWEEN the installs of a three-deep tower — each thread advances only while it is the innermost, sum pins the interleave"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (E.next)
                   (+ (handle E 50
                        ((next () s (resume s (+ s 5))))
                        (+ (E.next)
                           (+ (handle E 700
                                ((next () s (resume s (+ s 7))))
                                (E.next))
                              (E.next))))
                      (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 816 Int64))
  (call   main (: 0 Int64)) (output (: 806 Int64))
  (call   main (: -3 Int64)) (output (: 800 Int64)))
