(case "ha2 outer-draw feeds the INNER op, whose result feeds an OUTER op again — a three-hop cross-handler value chain"
  (input  (do
            (effect O (op next (-> Int64)) (op send (-> Int64 Int64)))
            (effect I (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1)))
                 (send (v) s (resume (+ v s) s)))
                (handle I 0
                  ((dbl (x) s (resume (* 2 x) s)))
                  (O.send (I.dbl (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -8 Int64)))
