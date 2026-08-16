(case "sg2 a stable PREFIX slice of a growing rope — String.slice (start,END) of the state per dispatch, prefix identical across growth"
  (input  (do
            (effect St (op grow (-> String)))
            (def (main (: n Int64))
              (handle St "ab"
                ((grow () s (resume (match (String.slice s 0 2) ((Some p) p) ((None) "?"))
                                    (String.concat s "cd"))))
                (let ((p1 (St.grow)))
                  (let ((p2 (St.grow)))
                    (if (= p1 p2) (String.byte-len (String.concat p1 p2)) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 4 Int64)))
