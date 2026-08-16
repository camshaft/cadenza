(case "sqm1 SQUARE-AND-MULTIPLY over the state — (base,acc) squares every dispatch, multiplies on 1-bits mod 1000, the n-bit's effect observed by a final read"
  (input  (do
            (effect S (op bit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 3 1)
                ((bit (b) st
                  (match st
                    ((tuple base acc)
                      (resume acc
                              (tuple (% (* base base) 1000)
                                     (if (= b 1) (% (* acc base) 1000) acc)))))))
                (let ((_a (S.bit 1)))
                  (let ((_b (S.bit 0)))
                    (let ((_c (S.bit 1)))
                      (let ((_d (S.bit n)))
                        (S.bit 0)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 243 Int64))
  (call   main (: 1 Int64)) (output (: 323 Int64)))
