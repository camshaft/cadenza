(case "or1 an open-row projector applied to TWO record widths where one field is a fresh DRAW — per-call-site slots under effect state"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (+ (get-x (record (= x (St.get))))
                   (* 100 (get-x (record (= a 9) (= x (St.get)) (= z 8)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 403 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
