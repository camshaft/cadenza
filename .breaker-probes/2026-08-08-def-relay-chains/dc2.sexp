(case "dc2 the relay call's ARGUMENT is a draw — the callee draws again and combines, argument-before-body order pinned"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (f (: x Int64)) (+ (* 10 x) (E.next)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (f (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))
