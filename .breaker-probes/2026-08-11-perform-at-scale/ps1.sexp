(case "ps1 a 100k-iteration tail loop that PERFORMS every iteration — dispatch itself must run in constant stack"
  (input  (do
            (effect Ctr (op next (-> Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (< n 1) acc (loop (- n 1) (+ acc (Ctr.next)))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next () s (resume s (+ s 1))))
                (loop n 0)))
            (export main)))
  (call   main (: 100000 Int64)) (output (: 4999950000 Int64))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
