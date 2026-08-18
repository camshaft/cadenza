(case "dbf1 a FOREIGN LEVY IN EACH REPLAY'S ANSWER ARGUMENT — both sequential resumes levy the outer handler while building their answers so the outer counter advances TWICE per dispatch once per replay, the surviving second replay carries the SECOND levy's value proving the discarded replay's levy still fired, and the seed shifts both levies together"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5))))
                (handle E (: 1 Int64)
                  ((tick () s
                    (do (resume (+ s (T.levy)) (+ s 1))
                        (resume (+ s (T.levy)) (+ s 2)))))
                  (E.tick))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))
