(case "id1 handling an ALREADY-DISCHARGED effect: outer handle of an effect the inner fully consumed"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 999
                ((a (u) s (resume (- 0 s) s)))
                (handle St n
                  ((a (u) s (resume s (+ s 1))))
                  (+ (St.a) (St.a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
