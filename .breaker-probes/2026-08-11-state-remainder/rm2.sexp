(case "rm2 a remainder-terminated hunt with a BOUNDED fallback — the walk stops at the first multiple of seven or exhausts its budget"
  (input  (do
            (effect S (op next (-> Int64)))
            (def (hunt (: k Int64))
              (let ((d (S.next)))
                (if (= (% d 7) 0) (* 100 d) (if (< k 1) -999 (hunt (- k 1))))))
            (def (main (: n Int64))
              (handle S n
                ((next () s (resume s (+ s 2))))
                (hunt 20)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 700 Int64))
  (call   main (: 1 Int64)) (output (: 700 Int64))
  (call   main (: 6 Int64)) (output (: 1400 Int64)))
