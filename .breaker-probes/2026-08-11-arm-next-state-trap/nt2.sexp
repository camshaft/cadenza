(case "nt2 a LAZY next-state trap — the trapping next-state is computed only when a LATER dispatch needs it, and no later dispatch comes"
  (input  (do
            (effect St (op step (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((step () s (resume s (/ 100 (- s 4)))))
                (St.step)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4 Int64))
  (call   main (: 6 Int64)) (output (: 6 Int64)))
