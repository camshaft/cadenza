(case "gp1 a PERFORMING guard on a wildcard pattern — hit and miss arms both read the guard-advanced state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (match k
                    ((guard _x (> (St.next) 6)) (+ 100 (St.next)))
                    (_o (* 10 (St.next)))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 108 Int64))
  (call   main (: 2 Int64)) (output (: 40 Int64)))
