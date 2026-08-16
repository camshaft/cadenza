(case "ur1 an effectful closure on the UNTAKEN branch of a const-folded if does not false-reject"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main (: k Int64))
              (host (ask)
                (if true (+ k 1) ((fn ((: x Int64)) (+ x (ask.ask))) k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
