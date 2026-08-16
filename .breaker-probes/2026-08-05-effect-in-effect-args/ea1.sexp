(case "ea1 a perform whose ARG is a MATCH over another perform ((St.put (match (St.get) ...)))"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1)))
                 (put (v) s (resume (* v 10) s)))
                (St.put (match (St.get)
                          (5 100)
                          (_ -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1000 Int64)))
