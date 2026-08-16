(case "rw7 a PARTIAL record pattern over a performing literal — does the unbound field still re-eval"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (match (record (a (St.next)) (b (St.next)))
                            ((record (a x)) (* x 10))))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))
