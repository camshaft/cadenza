(case "cd1 a closure CAPTURES a draw then is called after LATER draws — the captured value must not re-read state"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((d1 (St.get)))
                  (let ((f (fn (k) (+ (* 100 d1) k))))
                    (let ((d2 (St.get)))
                      (+ (f d2) (* 10000 (St.get))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 50304 Int64))
  (call   main (: 0 Int64)) (output (: 20001 Int64)))
