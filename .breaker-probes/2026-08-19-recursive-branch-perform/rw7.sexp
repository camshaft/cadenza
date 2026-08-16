(case "rw7 control: perform in a let-init then if scrutinizes it (sum-down shape)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0) 0
                (let ((i (St.get)))
                  (if (> i 0) (+ i (walk (- n 1))) (walk (- n 1))))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 6 Int64)))
