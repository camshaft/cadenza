(case "rp2 a RECORD op argument read by field beside an inner sum-state match — the record-payload face of the per-dispatch arm"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> (Record (: k Int64) (: w Int64)) Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (resume (* (. c k) (. c w)) (Mode.Run (* (. c k) (. c w)))))
                    ((Mode.Run j) (resume (+ j (* (. c k) (. c w))) (Mode.Run j))))))
                (+ (M.step (record (= k (+ 10 n)) (= w 2)))
                   (M.step (record (= k 3) (= w 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 52 Int64)))
