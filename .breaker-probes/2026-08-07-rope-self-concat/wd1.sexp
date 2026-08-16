(case "wd1 a perform-derived rope SELF-concatenated — the shared subtree measures correctly"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((w (String.concat "x" (if (> (St.next) 4) "big" "sm"))))
                  (let ((again (String.concat w w)))
                    (+ (* 10 (String.byte-len again)) (String.byte-len w))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 84 Int64)))
