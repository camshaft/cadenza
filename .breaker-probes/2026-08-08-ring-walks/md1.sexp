(case "md1 a MODULAR ring state — the thread walks a size-7 ring with stride 3, entry point reduced mod 7 at the seed"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 7)
                ((step () s (resume s (% (+ s 3) 7))))
                (+ (* 100 (E.step)) (+ (* 10 (E.step)) (E.step)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 251 Int64))
  (call   main (: 6 Int64)) (output (: 625 Int64))
  (call   main (: 13 Int64)) (output (: 625 Int64)))
