(case "s2 a String host RESULT is read twice by the guest (consuming-op discipline at the boundary)"
  (input  (do
            (effect io (op fetch (-> Unit String)))
            (def (main (: k Int64))
              (host (io)
                (let ((s (io.fetch unit)))
                  (+ (String.byte-len s)
                     (* 100 (String.scalar-len s))))))
            (export main)))
  (host-responses (respond io.fetch (: "héllo" String)))
  (host-calls (call io.fetch))
  (call   main (: 0 Int64)) (output (: 506 Int64)))
