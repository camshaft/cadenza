(case "su4 a sum variant carrying a HEAP list — Empty seeds on first put, Full grows the payload thereafter"
  (input  (do
            (type Buf (Empty) (Full (List Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle B (Empty)
                ((put (v) s (match s
                              ((Empty) (resume 0 (Full (list v))))
                              ((Full xs) (resume (List.len xs) (Full (List.push xs v)))))))
                (+ (B.put n) (+ (* 10 (B.put 7)) (* 100 (B.put 8))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 210 Int64)))
