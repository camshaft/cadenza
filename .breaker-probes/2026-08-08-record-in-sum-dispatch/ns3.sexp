(case "ns3 TWO record-payload variants selected by state parity — each arm projects its own record shape"
  (input  (do
            (type Shape
              (Pt (Record (: x Int64) (: y Int64)))
              (Ln (Record (: a Int64) (: b Int64) (: len Int64))))
            (effect E (op make (-> Shape)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (if (= (% s 2) 0)
                                        (Shape.Pt (record (x s) (y (+ s 1))))
                                        (Shape.Ln (record (a s) (b (* 2 s)) (len (* 3 s)))))
                                    (+ s 5)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.make)
                           ((Shape.Pt r) (+ (* 100 (. r x)) (* 10 (. r y))))
                           ((Shape.Ln r) (+ (. r a) (+ (. r b) (. r len))))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4505 Int64))
  (call   main (: 3 Int64)) (output (: 185 Int64))
  (call   main (: 0 Int64)) (output (: 105 Int64))
  (call   main (: -5 Int64)) (output (: -295 Int64)))
