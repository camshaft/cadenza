(case "pyz4 the TOLL ATTACHES TO ONLY THE SECOND REPLAY — the first resume stands bare and discarded while the second carries a thousandfold post-resume toll, the toll fires once on the surviving replay's outcome and never for the bare one, and a lowering sharing one toll shape across both replay sites of the arm double-charges"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (+ (resume (+ s 10) (+ s 2)) (* 1000 (+ s 1))))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2011 Int64))
  (call   main (: 0 Int64)) (output (: 1010 Int64)))
