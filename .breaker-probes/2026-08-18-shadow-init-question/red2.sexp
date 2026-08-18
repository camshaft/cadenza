(case "red2 the SHADOW-INIT DRAW with a PURE inner region — the control for the composition law: with no shadowed draw the outer toll fires exactly once per outer frame and the value matches the naive deep-handler model"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (handle E (* 10 (E.tick))
                  ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                  (: 7 Int64))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2007 Int64))
  (call   main (: 0 Int64)) (output (: 1007 Int64)))
