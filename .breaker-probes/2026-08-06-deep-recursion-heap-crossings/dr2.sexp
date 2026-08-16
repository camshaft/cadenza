(case "dr2 a handler state GROWS a list across 100 dispatches — the accumulated spine reads back intact"
  (input  (do
            (effect Log (op note (-> Int64 Int64)))
            (def (loop (: i Int64) (: acc Int64))
              (if (> i 100) acc
                (loop (+ i 1) (+ acc (Log.note i)))))
            (def (main (: n Int64))
              (handle Log (list)
                ((note (v) s (resume (List.len s) (List.push s v))))
                (+ (* 100 (loop 1 0))
                   0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 495000 Int64)))
