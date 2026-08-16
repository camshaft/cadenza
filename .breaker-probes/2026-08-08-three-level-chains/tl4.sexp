(case "tl4 SAME effect handled at two depths — the inner handle shadows for its extent, the outer thread resumes after it closes"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle E 50
                     ((next () s (resume s (+ s 5))))
                     (E.next))
                   (* 10 (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64))
  (call   main (: -4 Int64)) (output (: 10 Int64)))
