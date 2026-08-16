(case "ic5 a recursive LOOP whose exit condition draws per iteration — the iteration count is state-determined"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (spin (: acc Int64))
              (if (> (St.next) 20)
                  acc
                  (spin (+ acc 1))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 5))))
                (spin 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: 21 Int64)) (output (: 0 Int64))
  (call   main (: 11 Int64)) (output (: 2 Int64)))
