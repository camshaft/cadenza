(case "sn1 an arm that DISPATCHES on a String op-arg by equality (string-keyed command routing)"
  (input  (do
            (effect Cmd (op run (-> String Int64)))
            (def (main (: n Int64))
              (handle Cmd n
                ((run (name) s
                  (resume (if (= name "add") (+ s 1)
                            (if (= name "mul") (* s 2) -1))
                          s)))
                (+ (* 100 (Cmd.run "add")) (+ (* 10 (Cmd.run "mul")) (Cmd.run "nope")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 699 Int64)))
