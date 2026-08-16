(case "hr1 a HANDLE expression as a record-literal FIELD value — the region's result sits beside a pure field"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r (record (a (handle St n
                                    ((next () s (resume s (+ s 1))))
                                    (+ (St.next) (St.next))))
                               (b 7))))
                (+ (* 100 (. r a)) (. r b))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1107 Int64))
  (call   main (: 0 Int64)) (output (: 107 Int64)))
