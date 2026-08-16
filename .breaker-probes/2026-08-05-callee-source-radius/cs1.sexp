(case "cs1 closure from a RECORD FIELD + perform-fed application"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (record (dbl (fn ((: x Int64)) (* x 2))) (big (fn ((: x Int64)) (+ x 1000)))))
                (handle St n
                  ((feed (u) s (resume s (+ s 1))))
                  (ops.big (St.feed)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1005 Int64)))
