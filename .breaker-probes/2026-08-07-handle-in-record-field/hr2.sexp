(case "hr2 a HANDLE expression as a middle LIST element — the region's result sits between pure elements"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (el (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) 0)))
            (def (main (: n Int64))
              (let ((xs (list 3
                              (handle St n
                                ((next () s (resume s (* s 2))))
                                (+ (St.next) (St.next)))
                              9)))
                (+ (el xs 0) (+ (* 10 (el xs 1)) (* 100 (el xs 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1053 Int64))
  (call   main (: 1 Int64)) (output (: 933 Int64)))
