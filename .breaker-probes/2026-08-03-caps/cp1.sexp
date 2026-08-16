(case "cp1 a delegated effect reached ONLY through a mutually-recursive pair fires"
  (input  (do
            (effect io (op ping (-> Int64 Int64)))
            (def (even-walk (: n Int64))
              (if (= n 0) (io.ping 0) (odd-walk (- n 1))))
            (def (odd-walk (: n Int64))
              (if (= n 0) 99 (even-walk (- n 1))))
            (def (main (: n Int64))
              (host (io) (even-walk n)))
            (export main)))
  (host-responses (respond io.ping (: 42 Int64)))
  (host-calls (call io.ping))
  (call   main (: 4 Int64)) (output (: 42 Int64)))
