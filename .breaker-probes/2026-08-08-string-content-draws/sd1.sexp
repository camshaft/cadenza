(case "sd1 a draw COUNT drives string repetition through a recursion — String.byte-len pins how many times the thread said to concat"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (rep (: k Int64) (: acc String))
              (if (<= k 0) acc (rep (- k 1) (String.concat acc "ab"))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((k (+ (% (E.next) 3) 1)))
                  (+ (* 100 (String.byte-len (rep k "")))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64))
  (call   main (: 1 Int64)) (output (: 401 Int64))
  (call   main (: 2 Int64)) (output (: 601 Int64)))
