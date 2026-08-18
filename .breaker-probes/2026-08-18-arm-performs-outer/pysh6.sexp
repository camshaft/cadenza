(case "pysh6 a TRIPLE SHADOW CHAIN OF SELF-PERFORMS — three handlers over the same effect stack and each inner arm's self-perform routes exactly ONE level out so the body's single draw cascades level by level to the outermost arm and the answers fold back in reverse, each level stamping its own state band, and any arm skipping a level or capturing its own region breaks a distinct digit band"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (resume (+ (* s 10) 1) (+ s 1))))
                (handle E (: 50 Int64)
                  ((tick () s (resume (+ s (E.tick)) (* s 2))))
                  (handle E (: 700 Int64)
                    ((tick () s (resume (+ s (E.tick)) (+ s 1))))
                    (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 761 Int64))
  (call   main (: 0 Int64)) (output (: 751 Int64)))
