(case "bw4 the ARM bit-mixes its argument with the live state — low nibble from the arg, bits 4-5 stamped from the state"
  (input  (do
            (effect E (op tag (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag (x) s (resume (+ (& x 15) (<< (& s 3) 4)) (+ s 1))))
                (+ (* 100 (E.tag 9)) (E.tag 20))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2536 Int64))
  (call   main (: 0 Int64)) (output (: 920 Int64))
  (call   main (: 6 Int64)) (output (: 4152 Int64)))
