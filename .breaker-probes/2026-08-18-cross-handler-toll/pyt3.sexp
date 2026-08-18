(case "pyt3 the FOREIGN LEVY IN THE NEXT-STATE ARGUMENT — each inner tick answers its plain state but grows the state thread by an outer levy, the levies fire at dispatch feeding the NEXT dispatch's answer rather than this one's, and the seed surfaces only from the second answer onward so a levy hoisted into the answer or deferred past the state write shifts every row after the first"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5))))
                (handle E (: 1 Int64)
                  ((tick () s
                    (resume s (+ s (T.levy)))))
                  (let ((a (E.tick)))
                    (let ((b (E.tick)))
                      (+ a (* 10 b)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 21 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
