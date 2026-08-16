(case "dd2 a do-def block computes the RESUME VALUE — the def scope is arm-local, both dispatches rebuild it"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume (do (def d (* s 2)) (+ d 1)) (+ s 1))))
                (+ (St.get) (* 100 (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 907 Int64))
  (call   main (: 0 Int64)) (output (: 301 Int64)))
