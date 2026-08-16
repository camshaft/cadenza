(case "dd3 a do-def block computes the NEXT-STATE — the def-bound step feeds the state thread"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (do (def d (+ s 1)) (* d 2)))))
                (+ (St.get) (* 100 (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 803 Int64))
  (call   main (: 0 Int64)) (output (: 200 Int64)))
