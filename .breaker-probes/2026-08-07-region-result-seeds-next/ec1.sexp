(case "ec1 a region's RESULT seeds a second same-effect region with a DIFFERENT arm shape"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((total (handle St n
                             ((next (u) s (resume s (+ s 1))))
                             (+ (St.next) (St.next)))))
                (handle St total
                  ((next (u) s (resume s (* s 2))))
                  (+ (St.next) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64)))
