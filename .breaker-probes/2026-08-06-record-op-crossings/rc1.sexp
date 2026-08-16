(case "rc1 a record is built and consumed inside the arm (structural product per dispatch)"
  (input  (do
            (effect St (op fetch (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((fetch (id) s
                  (resume (match (record (x (* id 2)) (y (+ id 1)))
                            (r (+ (. r x) (. r y)))) s)))
                (St.fetch n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))
