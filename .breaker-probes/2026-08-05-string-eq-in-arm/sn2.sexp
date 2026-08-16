(case "sn2 the command name is ROPE-built at the perform site (rope-vs-flat equality inside arm dispatch)"
  (input  (do
            (effect Cmd (op run (-> String Int64)))
            (def (main (: n Int64))
              (handle Cmd n
                ((run (name) s (resume (if (= name "add") (+ s 1) -1) s)))
                (Cmd.run (String.concat "a" (String.concat "d" "d")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
