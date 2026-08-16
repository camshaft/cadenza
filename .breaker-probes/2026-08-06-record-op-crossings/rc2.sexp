(case "rc2 a RECORD op result ((Record (x Int64) (y Int64))) crosses resume; the body projects both fields"
  (input  (do
            (effect St (op fetch (-> Int64 (Record (x Int64) (y Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((fetch (id) s (resume (record (x (* id 2)) (y (+ id 1))) s)))
                (let ((r (St.fetch n)))
                  (+ (. r x) (. r y)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))
