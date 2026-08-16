(case "mo1 a TWO-op effect fully shadowed — the inner handler re-interprets BOTH ops with different arms"
  (input  (do
            (effect St (op get (-> Int64)) (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s s))
                 (bump () s (resume s (+ s 1))))
                (+ (handle St 50
                     ((get () s (resume (* s 2) s))
                      (bump () s (resume s (+ s 10))))
                     (+ (St.bump) (St.get)))
                   (St.get))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 177 Int64))
  (call   main (: 100 Int64)) (output (: 270 Int64)))
