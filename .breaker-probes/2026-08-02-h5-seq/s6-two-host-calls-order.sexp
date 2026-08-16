(case "s6 two host calls with a pure item BETWEEN them stay in order after the elision"
  (input  (do
            (effect io (op put (-> Int64 Int64)) (op get (-> Unit Int64)))
            (def (main (: d Int64))
              (host (io)
                (do (io.put 1)
                    (/ 100 d)
                    (io.put 2)
                    (io.get unit))))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)) (respond io.put (: 0 Int64)) (respond io.get (: 77 Int64)))
  (host-calls (call io.put) (call io.put) (call io.get))
  (call   main (: 0 Int64)) (output (: 77 Int64)))
