(case "rf2 performs INLINE in record fields fold (records are not a strict-ctor boundary)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (record (lo (St.next)) (hi (St.next)))))
                  (+ (* 100 (. r lo)) (. r hi)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
