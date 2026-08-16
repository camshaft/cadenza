(case "cp2 EQUALITY over two heap values each built by a perform ((= (mk (St.a)) (mk (St.a))) — unequal by advance)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (mk (: v Int64)) (list v v))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (+ (if (= (mk (St.a)) (mk (St.a))) 100 10)
                   (if (= (mk 3) (mk 3)) 1 1000))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
