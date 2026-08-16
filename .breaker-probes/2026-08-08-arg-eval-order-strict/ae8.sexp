(case "ae8 LIST literal of three draws — element positions carry the draw order into the structure"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((xs (list (E.next) (E.next) (E.next))))
                  (match (List.at xs 0)
                    ((Some a) (match (List.at xs 1)
                      ((Some b) (match (List.at xs 2)
                        ((Some c) (+ (* 100 a) (+ (* 10 b) c)))
                        ((None) 0)))
                      ((None) 0)))
                    ((None) 0)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))
