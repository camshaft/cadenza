(case "rw2 projection readout of the same record"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (record (a (St.next)) (b (St.next)))))
                  (+ (* 10 (. r a)) (. r b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
