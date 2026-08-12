(case "uwr1 a NARROW UInt8 handler state — wrapping-add accumulates modulo 256 through the thread, each dispatch answers the widened running value"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (UInt8.wrap n)
                ((add (v) s
                  (let ((s2 (UInt8.wrapping-add s (UInt8.wrap v))))
                    (resume (Int64.of s2) s2))))
                (let ((a (S.add 200)))
                  (let ((b (S.add 100)))
                    (+ (* 1000 a) b)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 44144 Int64))
  (call   main (: 0 Int64)) (output (: 200044 Int64)))
