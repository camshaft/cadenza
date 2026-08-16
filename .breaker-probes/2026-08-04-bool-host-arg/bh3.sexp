(case "bh3 a bool host RESULT drives a branch (the reverse direction)"
  (input  (do
            (effect io (op ok (-> Unit Bool)))
            (def (main (: n Int64))
              (host (io) (if (io.ok) (+ n 1) (- n 1))))
            (export main)))
  (host-responses (respond io.ok (: true Bool)))
  (host-calls (call io.ok))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))
