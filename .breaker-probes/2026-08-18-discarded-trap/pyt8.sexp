(case "pyt8 the SURVIVING REPLAY'S ANSWER ARGUMENT TRAPS — the second resume's argument divides by the state so the zero seed traps while BUILDING the replay that would win, value position beats discard elision, and the nonzero seed rides the clean quotient through the surviving replay"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (/ 60 s) (+ s 2)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 60 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))
