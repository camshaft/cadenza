(case "sf2 the inner arm's NEXT-STATE re-performs to the outer — the forward sits in the state-thread position, not the resume value"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (handle St 100
                  ((get () t (resume t (+ t (St.get)))))
                  (+ (St.get) (* 1000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103100 Int64))
  (call   main (: 0 Int64)) (output (: 100100 Int64)))
