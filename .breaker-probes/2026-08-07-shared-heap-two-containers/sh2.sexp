(case "sh2 pushing onto a SHARED perform-built list — the original stays len 2 beside the grown copy"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((shared (list (St.next) 100)))
                  (let ((grown (List.push shared (St.next))))
                    (+ (* 100 (List.len shared)) (List.len grown))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 203 Int64)))
