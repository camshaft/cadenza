(case "hc3 the SAME performing helper under two SEQUENTIAL handlers — each handle interprets its draws with its own arm and seed"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (twice) (+ (St.next) (St.next)))
            (def (main (: n Int64))
              (+ (handle St n
                   ((next () s (resume s (+ s 1))))
                   (twice))
                 (* 100 (handle St 7
                          ((next () s (resume s (* s 3))))
                          (twice)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2811 Int64))
  (call   main (: 0 Int64)) (output (: 2801 Int64)))
