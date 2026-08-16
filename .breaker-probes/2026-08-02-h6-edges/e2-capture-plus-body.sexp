(case "e2 a let-bound host call CAPTURED by a returned closure AND read in the same body fires once"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (tuple v (fn ((: x Int64)) (+ v x))))))
            (def (main (: k Int64))
              (match (mk)
                ((tuple direct f) (+ direct (* 100 (f k))))))
            (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (call   main (: 3 Int64)) (output (: 1007 Int64)))
