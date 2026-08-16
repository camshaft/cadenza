(case "cp2 no-call path through the same mutual pair makes no host call"
  (input  (do
            (effect io (op ping (-> Int64 Int64)))
            (def (even-walk (: n Int64))
              (if (= n 0) (io.ping 0) (odd-walk (- n 1))))
            (def (odd-walk (: n Int64))
              (if (= n 0) 99 (even-walk (- n 1))))
            (def (main (: n Int64))
              (host (io) (even-walk n)))
            (export main)))
  (host-responses)
  (host-calls)
  (call   main (: 3 Int64)) (output (: 99 Int64)))
