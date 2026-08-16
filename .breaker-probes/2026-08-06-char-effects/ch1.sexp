(case "ch1 a Char op RESULT crosses the resume boundary (scalar-at through a handler)"
  (input  (do
            (effect St (op pick (-> Int64 (Option Char))))
            (def (main (: n Int64))
              (handle St "hello"
                ((pick (i) s (resume (String.scalar-at s i) s)))
                (match (St.pick n)
                  ((Some c) (if (= c #\e) 1 0))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
