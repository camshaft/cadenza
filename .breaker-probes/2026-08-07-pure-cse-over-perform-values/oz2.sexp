(case "oz2 identical PURE reads of a perform-built list — safe to share, values agree"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((xs (list (St.next) (St.next))))
                  (+ (* 100 (List.len xs)) (List.len xs)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 202 Int64)))
