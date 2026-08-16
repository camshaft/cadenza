(case "bc1 Bytes.compact of a rope STATE inside an arm: compacted equals uncompacted by content"
  (input  (do
            (effect St (op probe (-> Unit Int64)))
            (def (build (: n Int64) (: acc Bytes))
              (if (= n 0) acc (build (- n 1) (Bytes.concat acc (Bytes.of (list 7))))))
            (def (main (: n Int64))
              (handle St (build n (Bytes.of (list)))
                ((probe (u) s
                  (resume (if (= (Bytes.compact s) s) (Bytes.len s) -1) s)))
                (St.probe)))
            (export main)))
  (call   main (: 30 Int64)) (output (: 30 Int64)))
