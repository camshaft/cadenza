(case "ic2 an INLINE performing if-condition — the taken branch's draw reads the condition-advanced state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (if (> (St.next) 4)
                    (+ 100 (St.next))
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 2 Int64)) (output (: -4 Int64)))
