(case "sh1 a perform-derived list SHARED by two tuples — both readers see one allocation's content"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((shared (list (St.next) 100)))
                  (let ((t1 (tuple shared 1)))
                    (let ((t2 (tuple shared 2)))
                      (+ (* 100 (match t1 ((tuple xs _k) (match (List.at xs 0) ((Some v) v) ((None _u) -1)))))
                         (match t2 ((tuple ys _k) (List.len ys)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 502 Int64)))
