(case "bh2 a bool arg BESIDE a scalar and a String (mixed-arity with the new marshal)"
  (input  (do
            (effect io (op log (-> Bool Int64 String Int64)))
            (def (main (: n Int64))
              (host (io) (io.log (= n 3) n "tag")))
            (export main)))
  (host-responses (respond io.log (: 42 Int64)))
  (host-calls (call io.log))
  (call   main (: 3 Int64))
  (output (: 42 Int64)))
