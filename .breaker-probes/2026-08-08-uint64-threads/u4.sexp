(case "u4 a UInt64 ABOVE Int64.max rides as a SUM payload — parity picks Lo/Hi, the Hi arm range-checks the high half"
  (input  (do
            (type UBox (Lo UInt64) (Hi UInt64))
            (effect E (op make (-> UBox)) (op probe (-> UInt64)))
            (def (main (: n UInt64))
              (handle E n
                ((make () s (resume (if (= (% s (: 2 UInt64)) (: 0 UInt64))
                                        (UBox.Lo s)
                                        (UBox.Hi (+ (: 9223372036854775808 UInt64) s)))
                                    (+ s (: 1 UInt64))))
                 (probe () s (resume s s)))
                (+ (* (: 10 UInt64) (match (E.make)
                                      ((UBox.Lo v) (+ (: 100 UInt64) v))
                                      ((UBox.Hi v) (if (>= v (: 9223372036854775808 UInt64)) (: 1 UInt64) (: 9 UInt64)))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 UInt64)) (output (: 1041 UInt64))
  (call   main (: 3 UInt64)) (output (: 11 UInt64)))
