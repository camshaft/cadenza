(case "mo2 the arm matches a RECORD op-arg and threads a FIELD into the next state"
  (input  (do
            (effect St (op log (-> (Record (tag Int64) (val Int64)) Int64)) (op sum (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((log (r) s (resume (. r tag) (+ s (. r val))))
                 (sum (u) s (resume s s)))
                (+ (* 100 (St.log (record (tag 1) (val n))))
                   (+ (* 10 (St.log (record (tag 2) (val (* n 10)))))
                      (St.sum)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 175 Int64)))
