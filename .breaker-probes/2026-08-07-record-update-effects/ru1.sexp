(case "ru1 Record.with over a perform result — the ORIGINAL record survives beside the update"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (record (a (St.next)) (b 100))))
                  (let ((r2 (Record.with r #"b" (St.next))))
                    (+ (* 100 (. r2 a)) (+ (. r2 b) (. r b)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 606 Int64)))
