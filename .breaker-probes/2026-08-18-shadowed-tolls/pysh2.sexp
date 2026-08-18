(case "pysh2 the SHADOW UNINSTALLS BETWEEN OUTER DRAWS — an outer draw precedes the shadowing region and a THIRD draw follows it, the middle draw routes to the inner cheap-toll handler whose region closes before the third draw re-routes to the OUTER expensive-toll arm continuing the outer state where the first draw left it, and a shadow that leaks past its region steals both the third draw's state and its toll"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (+ (E.tick)
                   (+ (* 10 (handle E (: 50 Int64)
                              ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                              (E.tick)))
                      (* 1000 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 57501 Int64))
  (call   main (: 0 Int64)) (output (: 54500 Int64)))
