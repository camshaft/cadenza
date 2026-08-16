(case "hr3 host calls on BOTH branches of a runtime-selected if consume rows only on the taken path"
  (input  (do
            (effect io (op get (-> Unit Int64)) (op alt (-> Unit Int64)))
            (def (main (: n Int64))
              (host (io)
                (+ (if (> n 5) (io.get) (io.alt))
                   (io.get))))
            (export main)))
  (host-responses (respond io.alt (: 100 Int64))
                  (respond io.get (: 7 Int64)))
  (host-calls (call io.alt) (call io.get))
  (call   main (: 3 Int64))
  (output (: 107 Int64)))
