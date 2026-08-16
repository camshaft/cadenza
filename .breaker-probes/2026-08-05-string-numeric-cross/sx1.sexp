(case "sx1 MULTIBYTE String as handler state: scalar-len vs byte-len both observable across advances"
  (input  (do
            (effect St (op grow (-> Unit Int64)) (op bytes (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St "é"
                ((grow (u) s (resume (String.scalar-len s) (String.concat s "∀")))
                 (bytes (u) s (resume (String.byte-len s) s)))
                (+ (* 100 (St.grow)) (+ (* 10 (St.grow)) (St.bytes)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 128 Int64)))
