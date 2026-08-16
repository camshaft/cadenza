(case "rp3 the record payload carries a NESTED record — two-level field projection inside the arm beside the state match"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> (Record (: m (Record (: k Int64))) (: w Int64)) Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (resume (* (. (. c m) k) (. c w)) (Mode.Run (* (. (. c m) k) (. c w)))))
                    ((Mode.Run j) (resume (+ j (* (. (. c m) k) (. c w))) (Mode.Run j))))))
                (+ (M.step (record (= m (record (= k (+ 10 n)))) (= w 2)))
                   (M.step (record (= m (record (= k 3))) (= w 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 52 Int64)))
