(case "ed4 one shared performing helper called under TWO different live handlers — each call homes to its caller's region"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (helper (: k Int64))
              (+ (St.next) k))
            (def (region (: k Int64))
              (handle St (* k 10)
                ((next () s (resume s (+ s 1))))
                (+ (helper 500) (helper 6000))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (+ (St.next) (+ (region 3) (helper 70)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6643 Int64))
  (call   main (: 0 Int64)) (output (: 6633 Int64)))
