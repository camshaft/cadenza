(case "cd4 a captured draw crosses a HIGHER-ORDER def boundary — helper applies the closure twice with different args"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (twice f) (+ (f 1) (* 10 (f 2))))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((d (St.get)))
                  (+ (twice (fn (k) (+ (* 100 d) k))) (* 100000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 403321 Int64))
  (call   main (: 0 Int64)) (output (: 100021 Int64)))
