(case "se2 MEMBERSHIP of a draw decides a branch that draws again — the set gates the thread's continuation"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 10))))
                (let ((st (Set.of (list 10 20 30))))
                  (if (Set.contains st (E.next))
                      (+ 1000 (E.next))
                      (+ 2000 (E.next))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1020 Int64))
  (call   main (: 5 Int64)) (output (: 2015 Int64))
  (call   main (: 30 Int64)) (output (: 1040 Int64)))
