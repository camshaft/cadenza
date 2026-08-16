(case "cp1 comparison operators over perform results: (< (St.a) (St.a)) ordering observable"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (+ (if (< (St.a) (St.a)) 100 10)
                   (if (> (St.a) (St.a)) 1000 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64)))
