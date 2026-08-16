(case "hf2 a performing closure stored in a TUPLE, extracted, and applied — the effect still homes"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((pair (tuple 99 (fn ((: x Int64)) (+ x (io.get))))))
                  ((. pair 1) k))))
            (export main)))
  (host-responses (respond io.get (: 3 Int64)))
  (host-calls (call io.get))
  (call   main (: 10 Int64))
  (output (: 13 Int64)))
