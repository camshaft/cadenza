(case "slmin8 puts then ONE size call only"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1)
                                                (String.concat r (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put) (E.size))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5 Int64)))
